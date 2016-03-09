// Copyright 2015, 2016 Ethcore (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

use util::*;
use std::sync::atomic::AtomicBool;
use rayon::prelude::*;
use ethcore::views::{BlockView};
use ethcore::client::{BlockChainClient, BlockId};
use ethcore::block::*;
use ethcore::error::*;
use ethcore::transaction::SignedTransaction;
use transaction_queue::{TransactionQueue};

pub trait MinerService {
	fn status(&self) -> MinerStatus;

	fn import_transactions<T>(&self, transactions: Vec<SignedTransaction>, fetch_nonce: T)
		where T: Fn(&Address) -> U256;

	/// called when blocks are imported to chain, updates transactions queue
	fn chain_new_blocks(&self, chain: &BlockChainClient, good: &[H256], bad: &[H256], _retracted: &[H256]);

	/// Set the author that we will seal blocks as.
	fn set_author(&self, author: Address);

	/// Set the extra_data that we will seal blocks with.
	fn set_extra_data(&self, extra_data: Bytes);

	/// New chain head event. Restart mining operation.
	fn prepare_sealing(&self, chain: &BlockChainClient);

	/// Grab the `ClosedBlock` that we want to be sealed. Comes as a mutex that you have to lock.
	fn sealing_block(&self, chain: &BlockChainClient) -> &Mutex<Option<ClosedBlock>>;

	/// Submit `seal` as a valid solution for the header of `pow_hash`.
	/// Will check the seal, but not actually insert the block into the chain.
	fn submit_seal(&self, chain: &BlockChainClient, pow_hash: H256, seal: Vec<Bytes>) -> Result<(), Error>;
}

pub struct MinerStatus {
	pub transaction_queue_pending: usize,
	pub transaction_queue_future: usize,
}

pub struct Miner {
	transaction_queue: Mutex<TransactionQueue>,

	// for sealing...
	sealing_enabled: AtomicBool,
	sealing_block: Mutex<Option<ClosedBlock>>,
	author: RwLock<Address>,
	extra_data: RwLock<Bytes>,
}

impl Miner {
	/// Creates new instance of miner
	pub fn new() -> Miner {
		Miner {
			transaction_queue: Mutex::new(TransactionQueue::new()),
			sealing_enabled: AtomicBool::new(false),
			sealing_block: Mutex::new(None),
			author: RwLock::new(Address::new()),
			extra_data: RwLock::new(Vec::new()),
		}
	}

	/// Get the author that we will seal blocks as.
	fn author(&self) -> Address {
		*self.author.read().unwrap()
	}

	/// Get the extra_data that we will seal blocks wuth.
	fn extra_data(&self) -> Bytes {
		self.extra_data.read().unwrap().clone()
	}
}

impl MinerService for Miner {

	fn status(&self) -> MinerStatus {
		let status = self.transaction_queue.lock().unwrap().status();
		MinerStatus {
			transaction_queue_pending: status.pending,
			transaction_queue_future: status.future,
		}
	}

	fn import_transactions<T>(&self, transactions: Vec<SignedTransaction>, fetch_nonce: T)
		where T: Fn(&Address) -> U256 {
		let mut transaction_queue = self.transaction_queue.lock().unwrap();
		transaction_queue.add_all(transactions, fetch_nonce);
	}

	fn set_author(&self, author: Address) {
		*self.author.write().unwrap() = author;
	}


	fn set_extra_data(&self, extra_data: Bytes) {
		*self.extra_data.write().unwrap() = extra_data;
	}

	fn prepare_sealing(&self, chain: &BlockChainClient) {
		let no_of_transactions = 128;
		let transactions = self.transaction_queue.lock().unwrap().top_transactions(no_of_transactions);

		let b = chain.prepare_sealing(
			self.author(),
			self.extra_data(),
			transactions,
		);
		*self.sealing_block.lock().unwrap() = b;
	}

	fn sealing_block(&self, chain: &BlockChainClient) -> &Mutex<Option<ClosedBlock>> {
		if self.sealing_block.lock().unwrap().is_none() {
			self.sealing_enabled.store(true, atomic::Ordering::Relaxed);
			// TODO: Above should be on a timer that resets after two blocks have arrived without being asked for.
			self.prepare_sealing(chain);
		}
		&self.sealing_block
	}

	fn submit_seal(&self, chain: &BlockChainClient, pow_hash: H256, seal: Vec<Bytes>) -> Result<(), Error> {
		let mut maybe_b = self.sealing_block.lock().unwrap();
		match *maybe_b {
			Some(ref b) if b.hash() == pow_hash => {}
			_ => { return Err(Error::PowHashInvalid); }
		}

		let b = maybe_b.take();
		match chain.try_seal(b.unwrap(), seal) {
			Err(old) => {
				*maybe_b = Some(old);
				Err(Error::PowInvalid)
			}
			Ok(sealed) => {
				// TODO: commit DB from `sealed.drain` and make a VerifiedBlock to skip running the transactions twice.
				try!(chain.import_block(sealed.rlp_bytes()));
				Ok(())
			}
		}
	}

	fn chain_new_blocks(&self, chain: &BlockChainClient, good: &[H256], bad: &[H256], _retracted: &[H256]) {
		fn fetch_transactions(chain: &BlockChainClient, hash: &H256) -> Vec<SignedTransaction> {
			let block = chain
				.block(BlockId::Hash(hash.clone()))
				// Client should send message after commit to db and inserting to chain.
				.expect("Expected in-chain blocks.");
			let block = BlockView::new(&block);
			block.transactions()
		}

		{
			let good = good.par_iter().map(|h| fetch_transactions(chain, h));
			let bad = bad.par_iter().map(|h| fetch_transactions(chain, h));

			good.for_each(|txs| {
				let mut transaction_queue = self.transaction_queue.lock().unwrap();
				let hashes = txs.iter().map(|tx| tx.hash()).collect::<Vec<H256>>();
				transaction_queue.remove_all(&hashes, |a| chain.nonce(a));
			});
			bad.for_each(|txs| {
				// populate sender
				for tx in &txs {
					let _sender = tx.sender();
				}
				let mut transaction_queue = self.transaction_queue.lock().unwrap();
				transaction_queue.add_all(txs, |a| chain.nonce(a));
			});
		}

		if self.sealing_enabled.load(atomic::Ordering::Relaxed) {
			self.prepare_sealing(chain);
		}
	}
}
