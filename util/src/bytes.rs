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
//! fn main() {
//! 	bytes_convertable();
//! }
//! ```

use std::fmt;
use std::slice;
use std::ops::{Deref, DerefMut};
use hash::FixedHash;
use elastic_array::*;

/// Vector like object
pub trait VecLike<T> {
	/// Add an element to the collection
    fn vec_push(&mut self, value: T);

	/// Add a slice to the collection
    fn vec_extend(&mut self, slice: &[T]);
}

impl<T> VecLike<T> for Vec<T> where T: Copy {
	fn vec_push(&mut self, value: T) {
		Vec::<T>::push(self, value)
	}

	fn vec_extend(&mut self, slice: &[T]) {
		Vec::<T>::extend_from_slice(self, slice)
	}
}

macro_rules! impl_veclike_for_elastic_array {
	($from: ident) => {
		impl<T> VecLike<T> for $from<T> where T: Copy {
			fn vec_push(&mut self, value: T) {
				$from::<T>::push(self, value)
			}
			fn vec_extend(&mut self, slice: &[T]) {
				$from::<T>::append_slice(self, slice)

			}
		}
	}
}

impl_veclike_for_elastic_array!(ElasticArray16);
impl_veclike_for_elastic_array!(ElasticArray32);
impl_veclike_for_elastic_array!(ElasticArray1024);

/// Slie pretty print helper
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

/// Trait to allow a type to be pretty-printed in `format!`, where unoverridable
/// defaults cannot otherwise be avoided.
pub trait ToPretty {
	/// Convert a type into a derivative form in order to make `format!` print it prettily.
	fn pretty(&self) -> PrettySlice;
	/// Express the object as a hex string.
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

/// A byte collection reference that can either be a slice or a vector
pub enum BytesRef<'a> {
	/// This is a reference to a vector
	Flexible(&'a mut Bytes),
	/// This is a reference to a slice
	Fixed(&'a mut [u8])
}

impl<'a> Deref for BytesRef<'a> {
	type Target = [u8];

	fn deref(&self) -> &[u8] {
		match *self {
			BytesRef::Flexible(ref bytes) => bytes,
			BytesRef::Fixed(ref bytes) => bytes
		}
	}
}

impl <'a> DerefMut for BytesRef<'a> {
	fn deref_mut(&mut self) -> &mut [u8] {
		match *self {
			BytesRef::Flexible(ref mut bytes) => bytes,
			BytesRef::Fixed(ref mut bytes) => bytes
		}
	}
}

/// Vector of bytes
pub type Bytes = Vec<u8>;

/// Slice of bytes to underlying memory
pub trait BytesConvertable {
	// TODO: rename to as_slice
	/// Get the underlying byte-wise representation of the value.
	/// Deprecated - use `as_slice` instead.
	fn bytes(&self) -> &[u8];
	/// Get the underlying byte-wise representation of the value.
	fn as_slice(&self) -> &[u8] { self.bytes() }
	/// Get a copy of the underlying byte-wise representation.
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
