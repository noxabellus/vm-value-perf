#![allow(unused_imports, dead_code, non_snake_case, clippy::all)]

#![feature(test)]
extern crate test;
use test::Bencher;

use std::mem::transmute;


const N: usize = 1_000_000;
const X_NIL_RATE: usize = 13;
const Y_NIL_RATE: usize = 33;

fn make_vec<T, F: FnMut (usize) -> T> (mut f: F) -> Vec<T> {
  let mut out = Vec::with_capacity(N);

  for i in 0..N { out.push(f(i)) }

  out
}


mod aligned_tagged {
  use super::*;

  enum Value {
    Nil,
    Number(f64),
    Userdata(*mut ()),
    // ...
  }

  #[bench]
  fn bench_aligned (bencher: &mut Bencher) {
    let x = make_vec(|i| if i % X_NIL_RATE == 0 { Value::Nil } else { Value::Number(i as f64 * 1.92) });
    let y = make_vec(|i| if i % Y_NIL_RATE == 0 { Value::Nil } else { Value::Number(i as f64 * 3.13) });

    let mut results = make_vec(|_| Value::Nil);

    bencher.iter(|| {
      for i in 0..N {
        let (a, b, r) = unsafe { (
          x.get_unchecked(i),
          y.get_unchecked(i),
          results.get_unchecked_mut(i)
        ) };
        
        match (a, b) {
          (&Value::Number(a), &Value::Number(b)) => *r = Value::Number(a + b),
          _ => *r = Value::Nil
        }
      }
    })
  }
}


mod separated_type_info {
  use super::*;

  union ValueData {
    Nil: (),
    Number: f64,
    Userdata: *mut (),
    // ...
  }

  #[repr(u8)]
  enum ValueKind {
    Nil,
    Number,
    Userdata,
    // ...
  }

  #[bench]
  fn bench_separated_type_info (bencher: &mut Bencher) {
    let x = make_vec(|i| if i % X_NIL_RATE == 0 { ValueData { Nil: () } } else { ValueData { Number: i as f64 * 1.92 } });
    let x_ts = make_vec(|i| if i % X_NIL_RATE == 0 { ValueKind::Nil } else { ValueKind::Number });

    let y = make_vec(|i| if i % Y_NIL_RATE == 0 { ValueData { Nil: () } } else { ValueData { Number: i as f64 * 3.13 } });
    let y_ts = make_vec(|i| if i % Y_NIL_RATE == 0 { ValueKind::Nil } else { ValueKind::Number });

    let mut results = make_vec(|_| ValueData { Nil: () });
    let mut result_ts = make_vec(|_| ValueKind::Nil);

    bencher.iter(|| {
      for i in 0..N {
        let (a, at, b, bt, r, rt) = unsafe { (
          x.get_unchecked(i),
          x_ts.get_unchecked(i),
          y.get_unchecked(i),
          y_ts.get_unchecked(i),
          results.get_unchecked_mut(i),
          result_ts.get_unchecked_mut(i)
        ) };

        match (at, bt) {
          (ValueKind::Number, ValueKind::Number) => {
            *r = ValueData { Number: unsafe { a.Number + b.Number } };
            *rt = ValueKind::Number;
          },
          _ => {
            *r = ValueData { Nil: () };
            *rt = ValueKind::Nil;
          }
        }
      }
    })
  }
}


mod unaligned_tagged {
  use super::*;

  struct Value {
    discriminant: ValueKind,
    data: [u8; 8]
  }

  #[repr(u8)]
  enum ValueKind {
    Nil,
    Number,
    Userdata,
    // ...
  }

  impl Value {
    fn is_nil (&self) -> bool { matches!(self.discriminant, ValueKind::Nil) }
    fn is_number (&self) -> bool { matches!(self.discriminant, ValueKind::Number) }
    fn is_userdata (&self) -> bool { matches!(self.discriminant, ValueKind::Userdata) }

    
    unsafe fn as_number_unchecked (&self) -> f64 {
      f64::from_bits(u64::from_ne_bytes(self.data))
    }
    
    fn as_number (&self) -> Option<f64> {
      if self.is_number() {
        Some(unsafe { self.as_number_unchecked() })
      } else {
        None
      }
    }
    
    unsafe fn as_userdata_unchecked (&self) -> *mut () {
      transmute(u64::from_ne_bytes(self.data))
    }
    
    fn as_userdata (&self) -> Option<*mut ()> {
      if self.is_userdata() {
        Some(unsafe { self.as_userdata_unchecked() })
      } else {
        None
      }
    }


    fn from_nil () -> Self {
      Self { discriminant: ValueKind::Nil, data: 0u64.to_ne_bytes() }
    }
    
    fn from_number (data: f64) -> Self {
      Self { discriminant: ValueKind::Number, data: data.to_ne_bytes() }
    }
    
    fn from_userdata (data: *mut ()) -> Self {
      Self { discriminant: ValueKind::Userdata, data: (data as u64).to_ne_bytes() }
    }
  }

  #[bench]
  fn bench_unaligned (bencher: &mut Bencher) {
    let x = make_vec(|i| if i % X_NIL_RATE == 0 { Value::from_nil() } else { Value::from_number(i as f64 * 1.92) });
    let y = make_vec(|i| if i % Y_NIL_RATE == 0 { Value::from_nil() } else { Value::from_number(i as f64 * 3.13) });

    let mut results = make_vec(|_| Value::from_nil());

    bencher.iter(|| {
      for i in 0..N {
        let (a, b, r) = unsafe { (
          x.get_unchecked(i),
          y.get_unchecked(i),
          results.get_unchecked_mut(i)
        ) };

        match (&a.discriminant, &b.discriminant) {
          (ValueKind::Number, ValueKind::Number) => *r = Value::from_number(unsafe { a.as_number_unchecked() + b.as_number_unchecked() }),
          _ => *r = Value::from_nil()
        }
      }
    })
  }
}


mod nan_tagged {
  use super::*;

  struct Value(u64);

  #[repr(u64)]
  enum ValueKind {
    Number   = 0u64 << 48,
    Nil      = 1u64 << 48,
    Userdata = 2u64 << 48,
    // ...
  }

  impl Value {
    const NAN_MASK:  u64 = 0b_0_11111111111_1_000_000000000000000000000000000000000000000000000000;
    const TYPE_MASK: u64 = 0b_0_00000000000_0_111_000000000000000000000000000000000000000000000000;
    const DATA_MASK: u64 = 0b_0_00000000000_0_000_111111111111111111111111111111111111111111111111;

    fn get_nan_segment  (&self) -> u64 { self.0 & Self::NAN_MASK  }
    fn get_type_segment (&self) -> u64 { self.0 & Self::TYPE_MASK }
    fn get_data_segment (&self) -> u64 { self.0 & Self::DATA_MASK }

    fn is_nan (&self) -> bool {
      self.get_nan_segment() == Self::NAN_MASK
    }

    fn compare_type_segment (&self, discriminator: ValueKind) -> bool {
      self.get_type_segment() == (discriminator as u64)
    }


    fn is_number (&self) -> bool {
      !self.is_nan() | self.compare_type_segment(ValueKind::Number)
    }

    fn is_nil (&self) -> bool {
      self.is_nan() & self.compare_type_segment(ValueKind::Nil)
    }

    fn is_userdata (&self) -> bool {
      self.is_nan() & self.compare_type_segment(ValueKind::Userdata)
    }


    unsafe fn as_number_unchecked (&self) -> f64 { *(self as *const _ as *const f64) }

    fn as_number (&self) -> Option<f64> {
      if self.is_number() {
        Some(unsafe { self.as_number_unchecked() })
      } else {
        None
      }
    }

    unsafe fn as_userdata_unchecked (&self) -> *mut () { self.get_data_segment() as _ }

    fn as_userdata (&self) -> Option<*mut ()> {
      if self.is_userdata() {
        Some(unsafe { self.as_userdata_unchecked() })
      } else {
        None
      }
    }


    fn from_number (data: f64) -> Self { unsafe { transmute(data) } }
    fn from_nil () -> Self { Self(Self::NAN_MASK | ValueKind::Nil as u64) }
    fn from_userdata (data: *mut ()) -> Self { Self(data as u64 | Self::NAN_MASK | ValueKind::Userdata as u64) }
  }

  #[bench]
  fn bench_nan_tagged (bencher: &mut Bencher) {
    let x = make_vec(|i| if i % X_NIL_RATE == 0 { Value::from_nil() } else { Value::from_number(i as f64 * 1.92) });
    let y = make_vec(|i| if i % Y_NIL_RATE == 0 { Value::from_nil() } else { Value::from_number(i as f64 * 3.13) });

    let mut results = make_vec(|_| Value::from_nil());

    bencher.iter(|| {
      for i in 0..N {
        let (a, b, r) = unsafe { (
          x.get_unchecked(i),
          y.get_unchecked(i),
          results.get_unchecked_mut(i)
        ) };

        match (a.is_number(), b.is_number()) {
          (true, true) => *r = Value::from_number(unsafe { a.as_number_unchecked() + b.as_number_unchecked() }),
          _ => *r = Value::from_nil()
        }
      }
    })
  }
}