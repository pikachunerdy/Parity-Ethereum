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
use ethcore::views::{HeaderView, BlockView};
use ethcore::header::{BlockNumber, Header as BlockHeader};
use ethcore::client::{BlockChainClient, BlockStatus, BlockId, BlockChainInfo};
use ethcore::block::*;
use ethcore::error::*;
use ethcore::transaction::SignedTransaction;
use transaction_queue::TransactionQueue;

pub struct Miner {
	/// Transactions Queue
	transaction_queue: Mutex<TransactionQueue>,

	// for sealing...
	sealing_enabled: AtomicBool,
	sealing_block: Mutex<Option<ClosedBlock>>,
	author: RwLock<Address>,
	extra_data: RwLock<Bytes>,
}

impl Miner {
	pub fn new() -> Miner {
		Miner {
			transaction_queue: Mutex::new(TransactionQueue::new()),
			sealing_enabled: AtomicBool::new(false),
			sealing_block: Mutex::new(None),
			author: RwLock::new(Address::new()),
			extra_data: RwLock::new(Vec::new()),
		}
	}

	pub fn import_transactions<T>(&self, transactions: Vec<SignedTransaction>, fetch_nonce: T)
		where T: Fn(&Address) -> U256 {
		let mut transaction_queue = self.transaction_queue.lock().unwrap();
		transaction_queue.add_all(transactions, fetch_nonce);
	}

	/// Get the author that we will seal blocks as.
	pub fn author(&self) -> Address {
		self.author.read().unwrap().clone()
	}

	/// Set the author that we will seal blocks as.
	pub fn set_author(&self, author: Address) {
		*self.author.write().unwrap() = author;
	}

	/// Get the extra_data that we will seal blocks wuth.
	pub fn extra_data(&self) -> Bytes {
		self.extra_data.read().unwrap().clone()
	}

	/// Set the extra_data that we will seal blocks with.
	pub fn set_extra_data(&self, extra_data: Bytes) {
		*self.extra_data.write().unwrap() = extra_data;
	}

	/// New chain head event. Restart mining operation.
	fn prepare_sealing(&self, chain: &BlockChainClient) {
		let b = chain.prepare_sealing(self.author.read().unwrap().clone(), self.extra_data.read().unwrap().clone());
		*self.sealing_block.lock().unwrap() = b;
	}

	/// Grab the `ClosedBlock` that we want to be sealed. Comes as a mutex that you have to lock.
	pub fn sealing_block(&self, chain: &BlockChainClient) -> &Mutex<Option<ClosedBlock>> {
		if self.sealing_block.lock().unwrap().is_none() {
			self.sealing_enabled.store(true, atomic::Ordering::Relaxed);
			// TODO: Above should be on a timer that resets after two blocks have arrived without being asked for.
			self.prepare_sealing(chain);
		}
		&self.sealing_block
	}

	/// Submit `seal` as a valid solution for the header of `pow_hash`.
	/// Will check the seal, but not actually insert the block into the chain.
	pub fn submit_seal(&self, chain: &BlockChainClient, pow_hash: H256, seal: Vec<Bytes>) -> Result<(), Error> {
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

	/// called when block is imported to chain, updates transactions queue and propagates the blocks
	pub fn chain_new_blocks(&self, chain: &BlockChainClient, good: &[H256], bad: &[H256], _retracted: &[H256]) {
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
