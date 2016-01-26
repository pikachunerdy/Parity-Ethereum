//! Cost schedule and other parameterisations for the EVM.

/// Definition of the cost schedule and other parameterisations for the EVM.
pub struct Schedule {
	/// Does it support exceptional failed code deposit
	pub exceptional_failed_code_deposit: bool,
	/// Does it have a delegate cal
	pub have_delegate_call: bool,
	/// VM stack limit
	pub stack_limit: usize,
	/// Max number of nested calls/creates
	pub max_depth: usize,
	/// Gas prices for instructions in all tiers
	pub tier_step_gas: [usize; 8],
	/// Gas price for `EXP` opcode
	pub exp_gas: usize,
	/// Additional gas for `EXP` opcode for each byte of exponent
	pub exp_byte_gas: usize,
	/// Gas price for `SHA3` opcode
	pub sha3_gas: usize,
	/// Additional gas for `SHA3` opcode for each word of hashed memory
	pub sha3_word_gas: usize,
	/// Gas price for loading from storage
	pub sload_gas: usize,
	/// Gas price for setting new value to storage (`storage==0`, `new!=0`)
	pub sstore_set_gas: usize,
	/// Gas price for altering value in storage
	pub sstore_reset_gas: usize,
	/// Gas refund for `SSTORE` clearing (when `storage!=0`, `new==0`)
	pub sstore_refund_gas: usize,
	/// Gas price for `JUMPDEST` opcode
	pub jumpdest_gas: usize,
	/// Gas price for `LOG*`
	pub log_gas: usize,
	/// Additional gas for data in `LOG*`
	pub log_data_gas: usize,
	/// Additional gas for each topic in `LOG*`
	pub log_topic_gas: usize,
	/// Gas price for `CREATE` opcode
	pub create_gas: usize,
	/// Gas price for `*CALL*` opcodes
	pub call_gas: usize,
	/// Stipend for transfer for `CALL|CALLCODE` opcode when `value>0`
	pub call_stipend: usize,
	/// Additional gas required for value transfer (`CALL|CALLCODE`)
	pub call_value_transfer_gas: usize,
	/// Additional gas for creating new account (`CALL|CALLCODE`)
	pub call_new_account_gas: usize,
	/// Refund for SUICIDE
	pub suicide_refund_gas: usize,
	/// Gas for used memory
	pub memory_gas: usize,
	/// Coefficient used to convert memory size to gas price for memory
	pub quad_coeff_div: usize,
	/// Cost for contract length when executing `CREATE`
	pub create_data_gas: usize,
	/// Transaction cost
	pub tx_gas: usize,
	/// `CREATE` transaction cost
	pub tx_create_gas: usize,
	/// Additional cost for empty data transaction
	pub tx_data_zero_gas: usize,
	/// Aditional cost for non-empty data transaction
	pub tx_data_non_zero_gas: usize,
	/// Gas price for copying memory
	pub copy_gas: usize,
}

impl Schedule {
	/// Schedule for the Frontier-era of the Ethereum main net.
	pub fn new_frontier() -> Schedule {
		Self::new(false, false, 21000)
	}

	/// Schedule for the Homestead-era of the Ethereum main net.
	pub fn new_homestead() -> Schedule {
		Self::new(true, true, 53000)
	}

	fn new(efcd: bool, hdc: bool, tcg: usize) -> Schedule {
		Schedule{
			exceptional_failed_code_deposit: efcd,
			have_delegate_call: hdc,
			stack_limit: 1024,
			max_depth: 1024,
			tier_step_gas: [0, 2, 3, 5, 8, 10, 20, 0],
			exp_gas: 10,
			exp_byte_gas: 10,
			sha3_gas: 30,
			sha3_word_gas: 6,
			sload_gas: 50,
			sstore_set_gas: 20000,
			sstore_reset_gas: 5000,
			sstore_refund_gas: 15000,
			jumpdest_gas: 1,
			log_gas: 375,
			log_data_gas: 8,
			log_topic_gas: 375,
			create_gas: 32000,
			call_gas: 40,
			call_stipend: 2300,
			call_value_transfer_gas: 9000,
			call_new_account_gas: 25000,
			suicide_refund_gas: 24000,
			memory_gas: 3,
			quad_coeff_div: 512,
			create_data_gas: 200,
			tx_gas: 21000,
			tx_create_gas: tcg,
			tx_data_zero_gas: 4,
			tx_data_non_zero_gas: 68,
			copy_gas: 3,	
		}
	}
}
