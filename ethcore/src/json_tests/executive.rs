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

use super::test_common::*;
use state::*;
use executive::*;
use spec::*;
use engine::*;
use evm;
use evm::{Schedule, Ext, Factory, VMType, ContractCreateResult, MessageCallResult};
use ethereum;
use externalities::*;
use substate::*;
use tests::helpers::*;
use state_diff::StateDiff;
use ethjson;

struct TestEngineFrontier {
	vm_factory: Factory,
	spec: Spec,
	max_depth: usize
}

impl TestEngineFrontier {
	fn new(max_depth: usize, vm_type: VMType) -> TestEngineFrontier {
		TestEngineFrontier {
			vm_factory: Factory::new(vm_type),
			spec: ethereum::new_frontier_test(),
			max_depth: max_depth
		}
	}
}

impl Engine for TestEngineFrontier {
	fn name(&self) -> &str { "TestEngine" }
	fn spec(&self) -> &Spec { &self.spec }
	fn vm_factory(&self) -> &Factory { &self.vm_factory }
	fn schedule(&self, _env_info: &EnvInfo) -> Schedule {
		let mut schedule = Schedule::new_frontier();
		schedule.max_depth = self.max_depth;
		schedule
	}
}

#[derive(Debug, PartialEq)]
struct CallCreate {
	data: Bytes,
	destination: Option<Address>,
	gas_limit: U256,
	value: U256
}

impl From<ethjson::vm::Call> for CallCreate {
	fn from(c: ethjson::vm::Call) -> Self {
		let dst: Option<_> = c.destination.into();
		CallCreate {
			data: c.data.into(),
			destination: dst.map(Into::into),
			gas_limit: c.gas_limit.into(),
			value: c.value.into()
		}
	}
}

/// Tiny wrapper around executive externalities.
/// Stores callcreates.
struct TestExt<'a> {
	ext: Externalities<'a>,
	callcreates: Vec<CallCreate>,
	contract_address: Address
}

impl<'a> TestExt<'a> {
	fn new(state: &'a mut State,
			   info: &'a EnvInfo,
			   engine: &'a Engine,
			   depth: usize,
			   origin_info: OriginInfo,
			   substate: &'a mut Substate,
			   output: OutputPolicy<'a, 'a>,
			   address: Address) -> Self {
		TestExt {
			contract_address: contract_address(&address, &state.nonce(&address)),
			ext: Externalities::new(state, info, engine, depth, origin_info, substate, output),
			callcreates: vec![]
		}
	}
}

impl<'a> Ext for TestExt<'a> {
	fn storage_at(&self, key: &H256) -> H256 {
		self.ext.storage_at(key)
	}

	fn set_storage(&mut self, key: H256, value: H256) {
		self.ext.set_storage(key, value)
	}

	fn exists(&self, address: &Address) -> bool {
		self.ext.exists(address)
	}

	fn balance(&self, address: &Address) -> U256 {
		self.ext.balance(address)
	}

	fn blockhash(&self, number: &U256) -> H256 {
		self.ext.blockhash(number)
	}

	fn create(&mut self, gas: &U256, value: &U256, code: &[u8]) -> ContractCreateResult {
		self.callcreates.push(CallCreate {
			data: code.to_vec(),
			destination: None,
			gas_limit: *gas,
			value: *value
		});
		ContractCreateResult::Created(self.contract_address.clone(), *gas)
	}

	fn call(&mut self,
			gas: &U256,
			_sender_address: &Address,
			receive_address: &Address,
			value: Option<U256>,
			data: &[u8],
			_code_address: &Address,
			_output: &mut [u8]) -> MessageCallResult {
		self.callcreates.push(CallCreate {
			data: data.to_vec(),
			destination: Some(receive_address.clone()),
			gas_limit: *gas,
			value: value.unwrap()
		});
		MessageCallResult::Success(*gas)
	}

	fn extcode(&self, address: &Address) -> Bytes  {
		self.ext.extcode(address)
	}

	fn log(&mut self, topics: Vec<H256>, data: &[u8]) {
		self.ext.log(topics, data)
	}

	fn ret(&mut self, gas: &U256, data: &[u8]) -> Result<U256, evm::Error> {
		self.ext.ret(gas, data)
	}

	fn suicide(&mut self, refund_address: &Address) {
		self.ext.suicide(refund_address)
	}

	fn schedule(&self) -> &Schedule {
		self.ext.schedule()
	}

	fn env_info(&self) -> &EnvInfo {
		self.ext.env_info()
	}

	fn depth(&self) -> usize {
		0
	}

	fn inc_sstore_clears(&mut self) {
		self.ext.inc_sstore_clears()
	}
}

fn do_json_test(json_data: &[u8]) -> Vec<String> {
	let vms = VMType::all();
	vms
		.iter()
		.flat_map(|vm| do_json_test_for(vm, json_data))
		.collect()
}

fn do_json_test_for(vm_type: &VMType, json_data: &[u8]) -> Vec<String> {
	let tests = ethjson::vm::Test::load(json_data).unwrap();
	let mut failed = Vec::new();

	for (name, vm) in tests.into_iter() {
		println!("name: {:?}", name);
		let mut fail = false;

		let mut fail_unless = |cond: bool, s: &str | if !cond && !fail {
			failed.push(format!("[{}] {}: {}", vm_type, name, s));
			fail = true
		};

		let out_of_gas = vm.out_of_gas();
		let mut state_result = get_temp_state();
		let mut state = state_result.reference_mut();
		state.populate_from(From::from(vm.pre_state.clone()));
		let info = From::from(vm.env);
		let engine = TestEngineFrontier::new(1, vm_type.clone());
		let params = ActionParams::from(vm.transaction);

		let mut substate = Substate::new(false);
		let mut output = vec![];

		// execute
		let (res, callcreates) = {
			let mut ex = TestExt::new(
				&mut state,
				&info,
				&engine,
				0,
				OriginInfo::from(&params),
				&mut substate,
				OutputPolicy::Return(BytesRef::Flexible(&mut output), None),
				params.address.clone()
			);
			let evm = engine.vm_factory().create();
			let res = evm.exec(params, &mut ex);
			(res, ex.callcreates)
		};

		match res {
			Err(_) => fail_unless(out_of_gas, "didn't expect to run out of gas."),
			Ok(gas_left) => {
				fail_unless(!out_of_gas, "expected to run out of gas.");
				fail_unless(Some(gas_left) == vm.gas_left.map(Into::into), "gas_left is incorrect");
				let vm_output: Option<Vec<u8>> = vm.output.map(Into::into);
				fail_unless(Some(output) == vm_output, "output is incorrect");

				let diff = StateDiff::diff_pod(&state.to_pod(), &From::from(vm.post_state.unwrap()));
				fail_unless(diff.is_empty(), format!("diff should be empty!: {:?}", diff).as_ref());
				let calls: Option<Vec<CallCreate>> = vm.calls.map(|c| c.into_iter().map(From::from).collect());
				fail_unless(Some(callcreates) == calls, "callcreates does not match");
			}
		};
	}

	for f in &failed {
		println!("FAILED: {:?}", f);
	}

	failed
}

declare_test!{ExecutiveTests_vmArithmeticTest, "VMTests/vmArithmeticTest"}
declare_test!{ExecutiveTests_vmBitwiseLogicOperationTest, "VMTests/vmBitwiseLogicOperationTest"}
declare_test!{ExecutiveTests_vmBlockInfoTest, "VMTests/vmBlockInfoTest"}
 // TODO [todr] Fails with Signal 11 when using JIT
declare_test!{ExecutiveTests_vmEnvironmentalInfoTest, "VMTests/vmEnvironmentalInfoTest"}
declare_test!{ExecutiveTests_vmIOandFlowOperationsTest, "VMTests/vmIOandFlowOperationsTest"}
declare_test!{heavy => ExecutiveTests_vmInputLimits, "VMTests/vmInputLimits"}
declare_test!{ExecutiveTests_vmLogTest, "VMTests/vmLogTest"}
declare_test!{ExecutiveTests_vmPerformanceTest, "VMTests/vmPerformanceTest"}
declare_test!{ExecutiveTests_vmPushDupSwapTest, "VMTests/vmPushDupSwapTest"}
declare_test!{ExecutiveTests_vmSha3Test, "VMTests/vmSha3Test"}
declare_test!{ExecutiveTests_vmSystemOperationsTest, "VMTests/vmSystemOperationsTest"}
declare_test!{ExecutiveTests_vmtests, "VMTests/vmtests"}
