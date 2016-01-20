//! Unified interfaces for bytes operations on basic types
//!
//! # Examples
//! ```rust
//! extern crate ethcore_util as util;
//!
//! fn bytes_convertable() {
//! 	use util::bytes::BytesConvertable;
//!
//! 	let arr = [0; 5];
//! 	let slice: &[u8] = arr.bytes();
//! }
//!
//! fn to_bytes() {
//! 	use util::bytes::ToBytes;
//!
//! 	let a: Vec<u8> = "hello_world".to_bytes();
//! 	let b: Vec<u8> = 400u32.to_bytes();
//! 	let c: Vec<u8> = 0xffffffffffffffffu64.to_bytes();
//! }
//!
//! fn from_bytes() {
//! 	use util::bytes::FromBytes;
//!
//! 	let a = String::from_bytes(&[b'd', b'o', b'g']);
//! 	let b = u16::from_bytes(&[0xfa]);
//! 	let c = u64::from_bytes(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
//! }
//!
//! fn main() {
//! 	bytes_convertable();
//! 	to_bytes();
//! 	from_bytes();
//! }
//! ```

use std::mem;
use std::fmt;
use std::slice;
use std::cmp::Ordering;
use std::error::Error as StdError;
use std::ops::{Deref, DerefMut};
use uint::{Uint, U128, U256};
use hash::FixedHash;

/// TODO [Gav Wood] Please document me
pub struct PrettySlice<'a> (&'a [u8]);

impl<'a> fmt::Debug for PrettySlice<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in 0..self.0.len() {
			match i > 0 {
				true => { try!(write!(f, "·{:02x}", self.0[i])); },
				false => { try!(write!(f, "{:02x}", self.0[i])); },
			}
		}
		Ok(())
	}
}

impl<'a> fmt::Display for PrettySlice<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in 0..self.0.len() {
			try!(write!(f, "{:02x}", self.0[i]));
		}
		Ok(())
	}
}

/// TODO [Gav Wood] Please document me
pub trait ToPretty {
	/// TODO [Gav Wood] Please document me
	fn pretty(&self) -> PrettySlice;
	/// TODO [Gav Wood] Please document me
	fn to_hex(&self) -> String {
		format!("{}", self.pretty())
	}
}

impl<'a> ToPretty for &'a [u8] {
	fn pretty(&self) -> PrettySlice {
		PrettySlice(self)
	}
}

impl<'a> ToPretty for &'a Bytes {
	fn pretty(&self) -> PrettySlice {
		PrettySlice(self.bytes())
	}
}
impl ToPretty for Bytes {
	fn pretty(&self) -> PrettySlice {
		PrettySlice(self.bytes())
	}
}

/// TODO [debris] Please document me
pub enum BytesRef<'a> {
	/// TODO [debris] Please document me
	Flexible(&'a mut Bytes),
	/// TODO [debris] Please document me
	Fixed(&'a mut [u8])
}

impl<'a> Deref for BytesRef<'a> {
	type Target = [u8];

	fn deref(&self) -> &[u8] {
		match self {
			&BytesRef::Flexible(ref bytes) => bytes,
			&BytesRef::Fixed(ref bytes) => bytes
		}
	}
}

impl <'a> DerefMut for BytesRef<'a> {
	fn deref_mut(&mut self) -> &mut [u8] {
		match self {
			&mut BytesRef::Flexible(ref mut bytes) => bytes,
			&mut BytesRef::Fixed(ref mut bytes) => bytes
		}
	}
}

/// Vector of bytes
pub type Bytes = Vec<u8>;

/// Slice of bytes to underlying memory
pub trait BytesConvertable {
	// TODO: rename to as_slice
	/// TODO [Gav Wood] Please document me
	fn bytes(&self) -> &[u8];
	/// TODO [Gav Wood] Please document me
	fn as_slice(&self) -> &[u8] { self.bytes() }
	/// TODO [Gav Wood] Please document me
	fn to_bytes(&self) -> Bytes { self.as_slice().to_vec() }
}

impl<'a> BytesConvertable for &'a [u8] {
	fn bytes(&self) -> &[u8] { self }
}

impl BytesConvertable for Vec<u8> {
	fn bytes(&self) -> &[u8] { self }
}

macro_rules! impl_bytes_convertable_for_array {
	($zero: expr) => ();
	($len: expr, $($idx: expr),*) => {
		impl BytesConvertable for [u8; $len] {
			fn bytes(&self) -> &[u8] { self }
		}
		impl_bytes_convertable_for_array! { $($idx),* }
	}
}

// -1 at the end is not expanded
impl_bytes_convertable_for_array! {
		32, 31, 30, 29, 28, 27, 26, 25, 24, 23, 22, 21, 20, 19, 18, 17, 16,
		15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, -1
}

#[test]
fn bytes_convertable() {
	assert_eq!(vec![0x12u8, 0x34].bytes(), &[0x12u8, 0x34]);
	assert_eq!([0u8; 0].bytes(), &[]);
}

/// Converts given type to its shortest representation in bytes
///
/// TODO: optimise some conversations
pub trait ToBytes {
	/// TODO [Gav Wood] Please document me
	fn to_bytes(&self) -> Vec<u8>;
	/// TODO [Gav Wood] Please document me
	fn to_bytes_len(&self) -> usize { self.to_bytes().len() }
	/// TODO [debris] Please document me
	fn first_byte(&self) -> Option<u8> { self.to_bytes().first().map(|&x| { x })}
}

impl <'a> ToBytes for &'a str {
	fn to_bytes(&self) -> Vec<u8> {
		From::from(*self)
	}

	fn to_bytes_len(&self) -> usize { self.len() }
}

impl ToBytes for String {
	fn to_bytes(&self) -> Vec<u8> {
		let s: &str = self.as_ref();
		From::from(s)
	}

	fn to_bytes_len(&self) -> usize { self.len() }
}

impl ToBytes for u64 {
	fn to_bytes(&self) -> Vec<u8> {
		let mut res= vec![];
		let count = self.to_bytes_len();
		res.reserve(count);
		for i in 0..count {
			let j = count - 1 - i;
			res.push((*self >> (j * 8)) as u8);
		}
		res
	}

	fn to_bytes_len(&self) -> usize { 8 - self.leading_zeros() as usize / 8 }
}

impl ToBytes for bool {
	fn to_bytes(&self) -> Vec<u8> {
		vec![ if *self { 1u8 } else { 0u8 } ]
	}

	fn to_bytes_len(&self) -> usize { 1 }
}

macro_rules! impl_map_to_bytes {
	($from: ident, $to: ty) => {
		impl ToBytes for $from {
			fn to_bytes(&self) -> Vec<u8> { (*self as $to).to_bytes() }
			fn to_bytes_len(&self) -> usize { (*self as $to).to_bytes_len() }
		}
	}
}

impl_map_to_bytes!(usize, u64);
impl_map_to_bytes!(u16, u64);
impl_map_to_bytes!(u32, u64);

macro_rules! impl_uint_to_bytes {
	($name: ident) => {
		impl ToBytes for $name {
			fn to_bytes(&self) -> Vec<u8> {
				let mut res= vec![];
				let count = self.to_bytes_len();
				res.reserve(count);
				for i in 0..count {
					let j = count - 1 - i;
					res.push(self.byte(j));
				}
				res
			}
			fn to_bytes_len(&self) -> usize { (self.bits() + 7) / 8 }
		}
	}
}

impl_uint_to_bytes!(U256);
impl_uint_to_bytes!(U128);

impl <T>ToBytes for T where T: FixedHash {
	fn to_bytes(&self) -> Vec<u8> {
		let mut res: Vec<u8> = vec![];
		res.reserve(T::size());

		unsafe {
			use std::ptr;
			ptr::copy(self.bytes().as_ptr(), res.as_mut_ptr(), T::size());
			res.set_len(T::size());
		}

		res
	}
}

/// Error returned when FromBytes conversation goes wrong
#[derive(Debug, PartialEq, Eq)]
pub enum FromBytesError {
	/// TODO [debris] Please document me
	DataIsTooShort,
	/// TODO [debris] Please document me
	DataIsTooLong
}

impl StdError for FromBytesError {
	fn description(&self) -> &str { "from_bytes error" }
}

impl fmt::Display for FromBytesError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		fmt::Debug::fmt(&self, f)
	}
}

/// Alias for the result of FromBytes trait
pub type FromBytesResult<T> = Result<T, FromBytesError>;

/// Converts to given type from its bytes representation
///
/// TODO: check size of bytes before conversation and return appropriate error
pub trait FromBytes: Sized {
	/// TODO [debris] Please document me
	fn from_bytes(bytes: &[u8]) -> FromBytesResult<Self>;
}

impl FromBytes for String {
	fn from_bytes(bytes: &[u8]) -> FromBytesResult<String> {
		Ok(::std::str::from_utf8(bytes).unwrap().to_string())
	}
}

macro_rules! impl_uint_from_bytes {
	($to: ident) => {
		impl FromBytes for $to {
			fn from_bytes(bytes: &[u8]) -> FromBytesResult<$to> {
				match bytes.len() {
					0 => Ok(0),
					l if l <= mem::size_of::<$to>() => {
						let mut res = 0 as $to;
						for i in 0..l {
							let shift = (l - 1 - i) * 8;
							res = res + ((bytes[i] as $to) << shift);
						}
						Ok(res)
					}
					_ => Err(FromBytesError::DataIsTooLong)
				}
			}
		}
	}
}

impl FromBytes for bool {
	fn from_bytes(bytes: &[u8]) -> FromBytesResult<bool> {
		match bytes.len() {
			0 => Ok(false),
			1 => Ok(bytes[0] != 0),
			_ => Err(FromBytesError::DataIsTooLong),
		}
	}
}

//impl_uint_from_bytes!(u8);
impl_uint_from_bytes!(u16);
impl_uint_from_bytes!(u32);
impl_uint_from_bytes!(u64);
impl_uint_from_bytes!(usize);

macro_rules! impl_uint_from_bytes {
	($name: ident) => {
		impl FromBytes for $name {
			fn from_bytes(bytes: &[u8]) -> FromBytesResult<$name> {
				if bytes.len() <= $name::SIZE {
					Ok($name::from(bytes))
				} else {
					Err(FromBytesError::DataIsTooLong)
				}
			}
		}
	}
}

impl_uint_from_bytes!(U256);
impl_uint_from_bytes!(U128);

impl <T>FromBytes for T where T: FixedHash {
	fn from_bytes(bytes: &[u8]) -> FromBytesResult<T> {
		match bytes.len().cmp(&T::size()) {
			Ordering::Less => return Err(FromBytesError::DataIsTooShort),
			Ordering::Greater => return Err(FromBytesError::DataIsTooLong),
			Ordering::Equal => ()
		};

		unsafe {
			use std::{mem, ptr};

			let mut res: T = mem::uninitialized();
			ptr::copy(bytes.as_ptr(), res.as_slice_mut().as_mut_ptr(), T::size());

			Ok(res)
		}
	}
}

/// Simple trait to allow for raw population of a Sized object from a byte slice.
pub trait Populatable {
	/// Copies a bunch of bytes `d` to `self`, overwriting as necessary.
	///
	/// If `d` is smaller, zero-out the remaining bytes.
	fn populate_raw(&mut self, d: &[u8]) {
		let mut s = self.as_slice_mut();
		for i in 0..s.len() {
			s[i] = if i < d.len() {d[i]} else {0};
		}
	}

	/// Copies a bunch of bytes `d` to `self`, overwriting as necessary.
	///
	/// If `d` is smaller, will leave some bytes untouched.
	fn copy_raw(&mut self, d: &[u8]) {
		use std::io::Write;
		self.as_slice_mut().write(&d).unwrap();
	}

	/// Copies the raw representation of an object `d` to `self`, overwriting as necessary.
	///
	/// If `d` is smaller, zero-out the remaining bytes.
	fn populate_raw_from(&mut self, d: &BytesConvertable) { self.populate_raw(d.as_slice()); }

	/// Copies the raw representation of an object `d` to `self`, overwriting as necessary.
	///
	/// If `d` is smaller, will leave some bytes untouched.
	fn copy_raw_from(&mut self, d: &BytesConvertable) { self.copy_raw(d.as_slice()); }

	/// Get the raw slice for this object.
	fn as_slice_mut(&mut self) -> &mut [u8];
}

impl<T> Populatable for T where T: Sized {
	fn as_slice_mut(&mut self) -> &mut [u8] {
		use std::mem;
		unsafe {
			slice::from_raw_parts_mut(self as *mut T as *mut u8, mem::size_of::<T>())
		}
	}
}

impl<T> Populatable for [T] where T: Sized {
	fn as_slice_mut(&mut self) -> &mut [u8] {
		use std::mem;
		unsafe {
			slice::from_raw_parts_mut(self.as_mut_ptr() as *mut u8, mem::size_of::<T>() * self.len())
		}
	}
}

#[test]
fn fax_raw() {
	let mut x = [255u8; 4];
	x.copy_raw(&[1u8; 2][..]);
	assert_eq!(x, [1u8, 1, 255, 255]);
	let mut x = [255u8; 4];
	x.copy_raw(&[1u8; 6][..]);
	assert_eq!(x, [1u8, 1, 1, 1]);
}

#[test]
fn populate_raw() {
	let mut x = [255u8; 4];
	x.populate_raw(&[1u8; 2][..]);
	assert_eq!(x, [1u8, 1, 0, 0]);
	let mut x = [255u8; 4];
	x.populate_raw(&[1u8; 6][..]);
	assert_eq!(x, [1u8, 1, 1, 1]);
}

#[test]
fn populate_raw_dyn() {
	let mut x = [255u8; 4];
	x.populate_raw(&[1u8; 2][..]);
	assert_eq!(&x[..], [1u8, 1, 0, 0]);
	let mut x = [255u8; 4];
	x.populate_raw(&[1u8; 6][..]);
	assert_eq!(&x[..], [1u8, 1, 1, 1]);
}

#[test]
fn fax_raw_dyn() {
	let mut x = [255u8; 4];
	x.copy_raw(&[1u8; 2][..]);
	assert_eq!(&x[..], [1u8, 1, 255, 255]);
	let mut x = [255u8; 4];
	x.copy_raw(&[1u8; 6][..]);
	assert_eq!(&x[..], [1u8, 1, 1, 1]);
}

#[test]
fn populate_big_types() {
	use hash::*;
	let a = address_from_hex("ffffffffffffffffffffffffffffffffffffffff");
	let mut h = h256_from_u64(0x69);
	h.populate_raw_from(&a);
	assert_eq!(h, h256_from_hex("ffffffffffffffffffffffffffffffffffffffff000000000000000000000000"));
	let mut h = h256_from_u64(0x69);
	h.copy_raw_from(&a);
	assert_eq!(h, h256_from_hex("ffffffffffffffffffffffffffffffffffffffff000000000000000000000069"));
}
