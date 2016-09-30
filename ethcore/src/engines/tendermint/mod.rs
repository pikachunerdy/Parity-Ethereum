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

//! Tendermint BFT consensus engine with round robin proof-of-authority.

mod message;
mod timeout;
mod params;

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use common::*;
use rlp::{UntrustedRlp, View, encode};
use ethkey::{recover, public_to_address};
use account_provider::AccountProvider;
use block::*;
use spec::CommonParams;
use engines::{Engine, EngineError, ProposeCollect};
use evm::Schedule;
use io::IoService;
use self::message::ConsensusMessage;
use self::timeout::{TimerHandler, NextStep};
use self::params::TendermintParams;

#[derive(Debug)]
enum Step {
	Propose,
	Prevote(ProposeCollect),
	/// Precommit step storing the precommit vote and accumulating seal.
	Precommit(ProposeCollect, Seal),
	/// Commit step storing a complete valid seal.
	Commit(BlockHash, Seal)
}

pub type Height = usize;
pub type Round = usize;
pub type BlockHash = H256;

pub type AtomicMs = AtomicUsize;
type Seal = Vec<Bytes>;

/// Engine using `Tendermint` consensus algorithm, suitable for EVM chain.
pub struct Tendermint {
	params: CommonParams,
	our_params: TendermintParams,
	builtins: BTreeMap<Address, Builtin>,
	timeout_service: IoService<NextStep>,
	/// Consensus round.
	r: AtomicUsize,
	/// Consensus step.
	s: RwLock<Step>,
	/// Current step timeout in ms.
	timeout: AtomicMs,
	/// Used to swith proposer.
	proposer_nonce: AtomicUsize,
}

impl Tendermint {
	/// Create a new instance of Tendermint engine
	pub fn new(params: CommonParams, our_params: TendermintParams, builtins: BTreeMap<Address, Builtin>) -> Arc<Self> {
		let engine = Arc::new(
			Tendermint {
				params: params,
				timeout: AtomicUsize::new(our_params.timeouts.propose),
				our_params: our_params,
				builtins: builtins,
				timeout_service: IoService::<NextStep>::start().expect("Error creating engine timeout service"),
				r: AtomicUsize::new(0),
				s: RwLock::new(Step::Propose),
				proposer_nonce: AtomicUsize::new(0)
			});
		let handler = TimerHandler::new(Arc::downgrade(&engine));
		engine.timeout_service.register_handler(Arc::new(handler)).expect("Error creating engine timeout service");
		engine
	}

	fn proposer(&self) -> Address {
		let ref p = self.our_params;
		p.validators.get(self.proposer_nonce.load(AtomicOrdering::Relaxed)%p.validator_n).unwrap().clone()
	}

	fn is_proposer(&self, address: &Address) -> bool {
		self.proposer() == *address
	}

	fn is_validator(&self, address: &Address) -> bool {
		self.our_params.validators.contains(address)
	}

	fn new_vote(&self, proposal: BlockHash) -> ProposeCollect {
		ProposeCollect::new(proposal,
							self.our_params.validators.iter().cloned().collect(),
							self.threshold())
	}

	fn to_step(&self, step: Step) {
		let mut guard = self.s.try_write().unwrap();
		*guard = step;
	}

	fn to_propose(&self) {
		trace!(target: "tendermint", "step: entering propose");
		println!("step: entering propose");
		self.proposer_nonce.fetch_add(1, AtomicOrdering::Relaxed);
		self.to_step(Step::Propose);
	}

	fn propose_message(&self, message: UntrustedRlp) -> Result<Bytes, Error> {
		// Check if message is for correct step.
		match *self.s.try_read().unwrap() {
			Step::Propose => (),
			_ => try!(Err(EngineError::WrongStep)),
		}
		let proposal = try!(message.as_val());
		self.to_prevote(proposal);
		Ok(message.as_raw().to_vec())
	}

	fn to_prevote(&self, proposal: BlockHash) {
		trace!(target: "tendermint", "step: entering prevote");
		println!("step: entering prevote");
		// Proceed to the prevote step.
		self.to_step(Step::Prevote(self.new_vote(proposal)));
	}

	fn prevote_message(&self, sender: Address, message: UntrustedRlp) -> Result<Bytes, Error> {
		// Check if message is for correct step.
		let hash = match *self.s.try_write().unwrap() {
			Step::Prevote(ref mut vote) => {
				// Vote if message is about the right block.
				if vote.hash == try!(message.as_val()) {
					vote.vote(sender);
					// Move to next step is prevote is won.
					if vote.is_won() {
						// If won assign a hash used for precommit.
						vote.hash.clone()
					} else {
						// Just propoagate the message if not won yet.
						return Ok(message.as_raw().to_vec());
					}
				} else {
					try!(Err(EngineError::WrongVote))
				}
			},
			_ => try!(Err(EngineError::WrongStep)),
		};
		self.to_precommit(hash);
		Ok(message.as_raw().to_vec())
	}

	fn to_precommit(&self, proposal: BlockHash) {
		trace!(target: "tendermint", "step: entering precommit");
		println!("step: entering precommit");
		self.to_step(Step::Precommit(self.new_vote(proposal), Vec::new()));
	}

	fn precommit_message(&self, sender: Address, signature: H520, message: UntrustedRlp) -> Result<Bytes, Error> {
		// Check if message is for correct step.
		match *self.s.try_write().unwrap() {
			Step::Precommit(ref mut vote, ref mut seal) => {
				// Vote and accumulate seal if message is about the right block.
				if vote.hash == try!(message.as_val()) {
					if vote.vote(sender) { seal.push(encode(&signature).to_vec()); }
					// Commit if precommit is won.
					if vote.is_won() { self.to_commit(vote.hash.clone(), seal.clone()); }
					Ok(message.as_raw().to_vec())
				} else {
					try!(Err(EngineError::WrongVote))
				}
			},
			_ => try!(Err(EngineError::WrongStep)),
		}
	}

	/// Move to commit step, when valid block is known and being distributed.
	pub fn to_commit(&self, block_hash: H256, seal: Vec<Bytes>) {
		trace!(target: "tendermint", "step: entering commit");
		println!("step: entering commit");
		self.to_step(Step::Commit(block_hash, seal));
	}

	fn threshold(&self) -> usize {
		self.our_params.validator_n*2/3
	}

	fn next_timeout(&self) -> u64 {
		self.timeout.load(AtomicOrdering::Relaxed) as u64
	}
}

impl Engine for Tendermint {
	fn name(&self) -> &str { "Tendermint" }
	fn version(&self) -> SemanticVersion { SemanticVersion::new(1, 0, 0) }
	/// Possibly signatures of all validators.
	fn seal_fields(&self) -> usize { 2 }

	fn params(&self) -> &CommonParams { &self.params }
	fn builtins(&self) -> &BTreeMap<Address, Builtin> { &self.builtins }

	/// Additional engine-specific information for the user/developer concerning `header`.
	fn extra_info(&self, _header: &Header) -> HashMap<String, String> { hash_map!["signature".to_owned() => "TODO".to_owned()] }

	fn schedule(&self, _env_info: &EnvInfo) -> Schedule {
		Schedule::new_homestead()
	}

	fn populate_from_parent(&self, header: &mut Header, parent: &Header, gas_floor_target: U256, _gas_ceil_target: U256) {
		header.set_difficulty(parent.difficulty().clone());
		header.set_gas_limit({
			let gas_limit = parent.gas_limit().clone();
			let bound_divisor = self.our_params.gas_limit_bound_divisor;
			if gas_limit < gas_floor_target {
				min(gas_floor_target, gas_limit + gas_limit / bound_divisor - 1.into())
			} else {
				max(gas_floor_target, gas_limit - gas_limit / bound_divisor + 1.into())
			}
		});
	}

	/// Apply the block reward on finalisation of the block.
	/// This assumes that all uncles are valid uncles (i.e. of at least one generation before the current).
	fn on_close_block(&self, _block: &mut ExecutedBlock) {}

	/// Attempt to seal the block internally using all available signatures.
	///
	/// None is returned if not enough signatures can be collected.
	fn generate_seal(&self, block: &ExecutedBlock, _accounts: Option<&AccountProvider>) -> Option<Vec<Bytes>> {
		self.s.try_read().and_then(|s| match *s {
			Step::Commit(hash, ref seal) if hash == block.header().bare_hash() => Some(seal.clone()),
			_ => None,
		})
	}

	fn handle_message(&self, sender: Address, signature: H520, message: UntrustedRlp) -> Result<Bytes, Error> {
		let c: ConsensusMessage = try!(message.as_val());
		println!("{:?}", c);
		// Check if correct round.
		if self.r.load(AtomicOrdering::Relaxed) != try!(message.val_at(0)) {
			try!(Err(EngineError::WrongRound))
		}
		// Handle according to step.
		match try!(message.val_at(1)) {
			0u8 if self.is_proposer(&sender) => self.propose_message(try!(message.at(2))),
			1 if self.is_validator(&sender) => self.prevote_message(sender, try!(message.at(2))),
			2 if self.is_validator(&sender) => self.precommit_message(sender, signature, try!(message.at(2))),
			_ => try!(Err(EngineError::UnknownStep)),
		}
	}

	fn verify_block_basic(&self, header: &Header, _block: Option<&[u8]>) -> Result<(), Error> {
		let seal_length = header.seal().len();
		if seal_length == self.seal_fields() {
			Ok(())
		} else {
			Err(From::from(BlockError::InvalidSealArity(
				Mismatch { expected: self.seal_fields(), found: seal_length }
			)))
		}
	}

	fn verify_block_unordered(&self, header: &Header, _block: Option<&[u8]>) -> Result<(), Error> {
		let to_address = |b: &Vec<u8>| {
			let sig: H520 = try!(UntrustedRlp::new(b.as_slice()).as_val());
			Ok(public_to_address(&try!(recover(&sig.into(), &header.bare_hash()))))
		};
		let validator_set = self.our_params.validators.iter().cloned().collect();
		let seal_set = try!(header
							.seal()
							.iter()
							.map(to_address)
							.collect::<Result<HashSet<_>, Error>>());
		if seal_set.intersection(&validator_set).count() <= self.threshold() {
			try!(Err(BlockError::InvalidSeal))
		} else {
			Ok(())
		}
	}

	fn verify_block_family(&self, header: &Header, parent: &Header, _block: Option<&[u8]>) -> Result<(), Error> {
		// we should not calculate difficulty for genesis blocks
		if header.number() == 0 {
			return Err(From::from(BlockError::RidiculousNumber(OutOfBounds { min: Some(1), max: None, found: header.number() })));
		}

		// Check difficulty is correct given the two timestamps.
		if header.difficulty() != parent.difficulty() {
			return Err(From::from(BlockError::InvalidDifficulty(Mismatch { expected: *parent.difficulty(), found: *header.difficulty() })))
		}
		let gas_limit_divisor = self.our_params.gas_limit_bound_divisor;
		let min_gas = parent.gas_limit().clone() - parent.gas_limit().clone() / gas_limit_divisor;
		let max_gas = parent.gas_limit().clone() + parent.gas_limit().clone() / gas_limit_divisor;
		if header.gas_limit() <= &min_gas || header.gas_limit() >= &max_gas {
			return Err(From::from(BlockError::InvalidGasLimit(OutOfBounds { min: Some(min_gas), max: Some(max_gas), found: header.gas_limit().clone() })));
		}
		Ok(())
	}

	fn verify_transaction_basic(&self, t: &SignedTransaction, _header: &Header) -> Result<(), Error> {
		try!(t.check_low_s());
		Ok(())
	}

	fn verify_transaction(&self, t: &SignedTransaction, _header: &Header) -> Result<(), Error> {
		t.sender().map(|_|()) // Perform EC recovery and cache sender
	}
}

#[cfg(test)]
mod tests {
	use common::*;
	use std::thread::sleep;
	use std::time::{Duration};
	use rlp::{UntrustedRlp, RlpStream, Stream, View, encode};
	use block::*;
	use tests::helpers::*;
	use account_provider::AccountProvider;
	use spec::Spec;
	use engines::{Engine, EngineError};
	use super::Tendermint;
	use super::params::TendermintParams;

	fn propose_default(engine: &Arc<Engine>, round: u8, proposer: Address) -> Result<Bytes, Error> {
		let mut s = RlpStream::new_list(3);
		let header = Header::default();
		s.append(&round).append(&0u8).append(&header.bare_hash());
		let drain = s.out();
		let propose_rlp = UntrustedRlp::new(&drain);

		engine.handle_message(proposer, H520::default(), propose_rlp)
	}

	fn vote_default(engine: &Arc<Engine>, round: u8, voter: Address) -> Result<Bytes, Error> {
		let mut s = RlpStream::new_list(3);
		let header = Header::default();
		s.append(&round).append(&1u8).append(&header.bare_hash());
		let drain = s.out();
		let vote_rlp = UntrustedRlp::new(&drain);

		engine.handle_message(voter, H520::default(), vote_rlp)
	}

	fn good_seal(header: &Header) -> Vec<Bytes> {
		let tap = AccountProvider::transient_provider();

		let mut seal = Vec::new();

		let v0 = tap.insert_account("0".sha3(), "0").unwrap();
		let sig0 = tap.sign_with_password(v0, "0".into(), header.bare_hash()).unwrap();
		seal.push(encode(&(&*sig0 as &[u8])).to_vec());

		let v1 = tap.insert_account("1".sha3(), "1").unwrap();
		let sig1 = tap.sign_with_password(v1, "1".into(), header.bare_hash()).unwrap();
		seal.push(encode(&(&*sig1 as &[u8])).to_vec());
		seal
	}

	fn default_block() -> Vec<u8> {
		vec![160, 39, 191, 179, 126, 80, 124, 233, 13, 161, 65, 48, 114, 4, 177, 198, 186, 36, 25, 67, 128, 97, 53, 144, 172, 80, 202, 75, 29, 113, 152, 255, 101]
	}

	#[test]
	fn has_valid_metadata() {
		let engine = Spec::new_test_tendermint().engine;
		assert!(!engine.name().is_empty());
		assert!(engine.version().major >= 1);
	}

	#[test]
	fn can_return_schedule() {
		let engine = Spec::new_test_tendermint().engine;
		let schedule = engine.schedule(&EnvInfo {
			number: 10000000,
			author: 0.into(),
			timestamp: 0,
			difficulty: 0.into(),
			last_hashes: Arc::new(vec![]),
			gas_used: 0.into(),
			gas_limit: 0.into(),
		});

		assert!(schedule.stack_limit > 0);
	}

	#[test]
	fn verification_fails_on_short_seal() {
		let engine = Spec::new_test_tendermint().engine;
		let header: Header = Header::default();

		let verify_result = engine.verify_block_basic(&header, None);

		match verify_result {
			Err(Error::Block(BlockError::InvalidSealArity(_))) => {},
			Err(_) => { panic!("should be block seal-arity mismatch error (got {:?})", verify_result); },
			_ => { panic!("Should be error, got Ok"); },
		}
	}

	#[test]
	fn verification_fails_on_wrong_signatures() {
		let engine = Spec::new_test_tendermint().engine;
		let mut header = Header::default();
		let tap = AccountProvider::transient_provider();

		let mut seal = Vec::new();

		let v1 = tap.insert_account("0".sha3(), "0").unwrap();
		let sig1 = tap.sign_with_password(v1, "0".into(), header.bare_hash()).unwrap();
		seal.push(encode(&(&*sig1 as &[u8])).to_vec());

		header.set_seal(seal.clone());

		// Not enough signatures.
		assert!(engine.verify_block_basic(&header, None).is_err());

		let v2 = tap.insert_account("101".sha3(), "101").unwrap();
		let sig2 = tap.sign_with_password(v2, "101".into(), header.bare_hash()).unwrap();
		seal.push(encode(&(&*sig2 as &[u8])).to_vec());

		header.set_seal(seal);

		// Enough signatures.
		assert!(engine.verify_block_basic(&header, None).is_ok());

		let verify_result = engine.verify_block_unordered(&header, None);

		// But wrong signatures.
		match verify_result {
			Err(Error::Block(BlockError::InvalidSeal)) => (),
			Err(_) => panic!("should be block seal-arity mismatch error (got {:?})", verify_result),
			_ => panic!("Should be error, got Ok"),
		}
	}

	#[test]
	fn seal_with_enough_signatures_is_ok() {
		let engine = Spec::new_test_tendermint().engine;
		let mut header = Header::default();

		let seal = good_seal(&header);
		header.set_seal(seal);

		// Enough signatures.
		assert!(engine.verify_block_basic(&header, None).is_ok());

		// And they are ok.
		assert!(engine.verify_block_unordered(&header, None).is_ok());
	}

	#[test]
	fn can_generate_seal() {
		let spec = Spec::new_test_tendermint();
		let ref engine = *spec.engine;
		let tender = Tendermint::new(engine.params().clone(), TendermintParams::default(), BTreeMap::new());

		let genesis_header = spec.genesis_header();
		let mut db_result = get_temp_journal_db();
		let mut db = db_result.take();
		spec.ensure_db_good(db.as_hashdb_mut()).unwrap();
		let last_hashes = Arc::new(vec![genesis_header.hash()]);
		let b = OpenBlock::new(engine, Default::default(), false, db, &genesis_header, last_hashes, Address::default(), (3141562.into(), 31415620.into()), vec![]).unwrap();
		let b = b.close_and_lock();

		tender.to_commit(b.hash(), good_seal(&b.header()));

		let seal = tender.generate_seal(b.block(), None).unwrap();
		assert!(b.try_seal(engine, seal).is_ok());
	}

	#[test]
	fn propose_step() {
		let engine = Spec::new_test_tendermint().engine;
		let tap = AccountProvider::transient_provider();
		let r = 0;

		let not_validator_addr = tap.insert_account("101".sha3(), "101").unwrap();
		assert!(propose_default(&engine, r, not_validator_addr).is_err());

		let not_proposer_addr = tap.insert_account("0".sha3(), "0").unwrap();
		assert!(propose_default(&engine, r, not_proposer_addr).is_err());

		let proposer_addr = tap.insert_account("1".sha3(), "1").unwrap();
		assert_eq!(default_block(), propose_default(&engine, r, proposer_addr).unwrap());

		assert!(propose_default(&engine, r, proposer_addr).is_err());
		assert!(propose_default(&engine, r, not_proposer_addr).is_err());
	}

	#[test]
	fn proposer_switching() {
		let engine = Spec::new_test_tendermint().engine;
		let tap = AccountProvider::transient_provider();

		// Currently not a proposer.
		let not_proposer_addr = tap.insert_account("0".sha3(), "0").unwrap();
		assert!(propose_default(&engine, 0, not_proposer_addr).is_err());

		sleep(Duration::from_millis(TendermintParams::default().timeouts.propose as u64));

		// Becomes proposer after timeout.
		assert_eq!(default_block(), propose_default(&engine, 0, not_proposer_addr).unwrap());
	}

	#[test]
	fn prevote_step() {
		let engine = Spec::new_test_tendermint().engine;
		let tap = AccountProvider::transient_provider();
		let r = 0;

		let v0 = tap.insert_account("0".sha3(), "0").unwrap();
		let v1 = tap.insert_account("1".sha3(), "1").unwrap();

		// Propose.
		assert!(propose_default(&engine, r, v1.clone()).is_ok());

		// Prevote.
		assert_eq!(default_block(), vote_default(&engine, r, v0.clone()).unwrap());

		assert!(vote_default(&engine, r, v0).is_err());
		assert!(vote_default(&engine, r, v1).is_err());
	}

	#[test]
	fn precommit_step() {
		let engine = Spec::new_test_tendermint().engine;
		let tap = AccountProvider::transient_provider();
		let r = 0;

		let v0 = tap.insert_account("0".sha3(), "0").unwrap();
		let v1 = tap.insert_account("1".sha3(), "1").unwrap();

		// Propose.
		assert!(propose_default(&engine, r, v1.clone()).is_ok());

		// Prevote.
		assert_eq!(default_block(), vote_default(&engine, r, v0.clone()).unwrap());

		assert!(vote_default(&engine, r, v0).is_err());
		assert!(vote_default(&engine, r, v1).is_err());
	}

	#[test]
	fn timeout_switching() {
		let tender = {
			let engine = Spec::new_test_tendermint().engine;
			Tendermint::new(engine.params().clone(), TendermintParams::default(), BTreeMap::new())
		};

		println!("Waiting for timeout");
		sleep(Duration::from_secs(10));
	}

	#[test]
	fn increments_round() {
		let spec = Spec::new_test_tendermint();
		let ref engine = *spec.engine;
		let def_params = TendermintParams::default();
		let tender = Tendermint::new(engine.params().clone(), def_params.clone(), BTreeMap::new());
		let header = Header::default();

		tender.to_commit(header.bare_hash(), good_seal(&header));

		sleep(Duration::from_millis(def_params.timeouts.commit as u64));

		match propose_default(&(tender as Arc<Engine>), 0, Address::default()) {
			Err(Error::Engine(EngineError::WrongRound)) => {},
			_ => panic!("Should be EngineError::WrongRound"),
		}
	}
}
