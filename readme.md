# VM Value Representation Benchmarks

In the interest of performance, I have benchmarked 4 different methods of representation for values inside VMs.
Of course, this is not representative of all situations encountered while interpreting a typical program; the chosen test case is a "best case scenario" simulating the VM performing math on linear arrays of numbers. That said, in order to add some variance to this test, a few nil values are mixed in to the input arrays.

- [Test Environment](#test-environment)
- [Setup](#setup)
  - [Aligned Tagged Union (Padded)](#aligned-tagged-union-padded)
    - [Structure](#structure)
    - [Benchmark](#benchmark)
  - [Separated Type Info (Parallel Arrays)](#separated-type-info-parallel-arrays)
    - [Structure](#structure-1)
    - [Benchmark](#benchmark-1)
  - [Unaligned Tagged Union (Packed)](#unaligned-tagged-union-packed)
    - [Structure](#structure-2)
    - [Benchmark](#benchmark-2)
  - ["Nan-tagged" Union (Double)](#nan-tagged-union-double)
    - [Structure](#structure-3)
    - [Benchmark](#benchmark-3)
- [Results](#results)
  - [Aligned Tagged](#aligned-tagged)
  - [Unaligned Tagged](#unaligned-tagged)
  - [Separated Type Info](#separated-type-info)
  - [Nan-tagged](#nan-tagged)
- [Conclusions](#conclusions)


## Test Environment

All of the benchmarks use the following constants:
```rust
const N: usize = 1_000_000;
const X_NIL_RATE: usize = 13;
const Y_NIL_RATE: usize = 33;
```

Also, the benchmarks use a simple vec builder helper:
```rust
fn make_vec<T, F: FnMut (usize) -> T> (mut f: F) -> Vec<T> {
  let mut out = Vec::with_capacity(N);

  for i in 0..N { out.push(f(i)) }

  out
}
```

When creating the test arrays, we will use variations on the repeating pattern shown here:
```rust
/// Fill with number-variant values except when we are at a multiple of `X_NIL_RATE`
let x = make_vec(|i| if i % X_NIL_RATE == 0 { nil } else { Number(i as f64 * 1.92) });

/// Fill with number-variant values except when we are at a multiple of `Y_NIL_RATE`
let y = make_vec(|i| if i % Y_NIL_RATE == 0 { nil } else { Number(i as f64 * 3.13) });

/// Create an "empty" array to store output of the test
let mut results = make_vec(|_| nil);
```


Note also that all the example values will include a `Userdata` variant, but it is not utilized in the benchmark. It is shown here to demonstrate how non-numeric data is accessed, as it is different in some implementations.



## Setup

The various methods are sorted in order of what I perceive to be the most complicated, from least to most.


### Aligned Tagged Union (Padded)

#### Structure

This is the most straight-forward representation that was tested. It uses a conventional tagged union, and is generally unremarkable:
```rust
enum Value {
  Nil,
  Number(f64),
  Userdata(*mut ()),
  // ...
}
```

The tradeoff here is simply in size, as since the variant field must be aligned to 8 bytes, it requires 7 bytes of padding after the discriminator.

With Rust's pattern matching and variant constructors, no additional code is required to support the benchmark.


#### Benchmark

```rust
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
```



### Separated Type Info (Parallel Arrays)


#### Structure

This is again a rather unremarkable data structure. Here, the discriminator and the data are stored separately in parallel arrays.

The data is stored in a raw union:
```rust
union ValueData {
  Nil: (),
  Number: f64,
  Userdata: *mut (),
  // ...
}
```

While the discriminant is a separate enum:
```rust
#[repr(u8)]
enum ValueKind {
  Nil,
  Number,
  Userdata,
  // ...
}
```

No additional code is required to support the benchmark.


#### Benchmark

```rust
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
```



### Unaligned Tagged Union (Packed)


#### Structure

For this version, the typical tagged union pattern is recreated, but our variant data is type-erased into a packed array of bytes:
```rust
struct Value {
  discriminant: ValueKind,
  data: [u8; 8]
}
```

This allows our overall Value to occupy a total of 9 bytes. An additional factor worth noting is that this allows many more type variants. The tradeoff here is that more functionality will be required to manipulate data, and we will have to perform unaligned loads and stores by transforming our data to and from byte arrays.

In order to access these values, a type discriminant enum and convenience methods were created:
```rust
#[repr(u8)]
pub enum ValueKind {
  Nil,
  Number,
  Userdata
  ...
}
```
```rust
fn is_nil (&self) -> bool { matches!(self.discriminant, ValueKind::Nil) }
fn is_number (&self) -> bool { matches!(self.discriminant, ValueKind::Number) }
fn is_userdata (&self) -> bool { matches!(self.discriminant, ValueKind::Userdata) }
```
```rust
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
```
```rust
fn from_nil () -> Self {
  Self { discriminant: ValueKind::Nil, data: 0u64.to_ne_bytes() }
}

fn from_number (data: f64) -> Self {
  Self { discriminant: ValueKind::Number, data: data.to_ne_bytes() }
}

fn from_userdata (data: *mut ()) -> Self {
  Self { discriminant: ValueKind::Userdata, data: (data as u64).to_ne_bytes() }
}
```


#### Benchmark

```rust
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
```



### "Nan-tagged" Union (Double)


#### Structure

In this version, the value is simply a newtype wrapper around a word-size integer:
```rust
struct Value(u64);
```

In order to represent the variant data, knowledge of the [IEEE-754 Specification](https://en.wikipedia.org/wiki/IEEE_754-1985#Representation_of_non-numbers) is utilized:
![IEE-754 Layout Diagram](https://upload.wikimedia.org/wikipedia/commons/thumb/a/a9/IEEE_754_Double_Floating_Point_Format.svg/2880px-IEEE_754_Double_Floating_Point_Format.svg.png)

In particular, the representation of `NaN` is specified as having all exponent bits set, and at least one fraction bit set.

In addition to this specification, on modern hardware only 48 bits of a 64 bit pointer are used. These are the right-most 48 bits in the above diagram. This leaves us 3 bits free in the middle to work with, yielding 8 possible variants for our "union".

In order to access these values, a type discriminant enum, masks and convenience methods were created:
```rust
#[repr(u64)]
enum ValueKind {
  Number   = 0u64 << 48,
  Nil      = 1u64 << 48,
  Userdata = 2u64 << 48,
  // ...
}
```
```rust
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
```
```rust
fn is_number (&self) -> bool {
  !self.is_nan() | self.compare_type_segment(ValueKind::Number)
}

fn is_nil (&self) -> bool {
  self.is_nan() & self.compare_type_segment(ValueKind::Nil)
}

fn is_userdata (&self) -> bool {
  self.is_nan() & self.compare_type_segment(ValueKind::Userdata)
}
```
```rust
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

```
```rust
fn from_number (data: f64) -> Self { unsafe { transmute(data) } }
fn from_nil () -> Self { Self(Self::NAN_MASK | ValueKind::Nil as u64) }
fn from_userdata (data: *mut ()) -> Self { Self(data as u64 | Self::NAN_MASK | ValueKind::Userdata as u64) }
```

Note that in the convenience method, bitwise operators are used to combine the booleans, rather than logical operators. This avoids short-circuiting evaluation behavior, and thus avoids branches.


#### Benchmark

```rust
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
```




## Results

The benchmark results are sorted in order of performance, from slowest to fastest. Each benchmark was run four times, with each time consisting of many iterations. Looking at the data across large numbers of runs at several levels of iteration provides the most unbiased representation possible. The numbers shown are all times, in nanoseconds.


### Aligned Tagged

| Runtimes         | Deviations    |
|------------------|---------------|
| 3,518,433 / iter | (+/- 71,447)  |
| 3,653,033 / iter | (+/- 41,844)  |
| 3,513,199 / iter | (+/- 45,572)  |
| 3,631,479 / iter | (+/- 115,099) |

| Runtime Data   |              | Deviation Data |           |
|----------------|--------------|----------------|-----------|
| Sum	           | 14,316,144   | Sum	           | 273,962   |
| Median	       | 3,574,956    | Median	       | 58,509.5  |
| Geometric Mean | 3,578,469.27 | Geometric Mean | 62,928.40 |
| Largest	       | 3,653,033    | Largest	       | 115,099   |
| Smallest	     | 3,513,199    | Smallest	     | 41,844    |
| Range	         | 139,834      | Range	         | 73,255    |


### Unaligned Tagged

| Runtimes         | Deviations   |
|------------------|--------------|
| 3,441,432 / iter | (+/- 37352)  |
| 3,477,079 / iter | (+/- 484310) |
| 3,439,800 / iter | (+/- 105023) |
| 3,471,989 / iter | (+/- 116979) |

| Runtime Data   |              | Deviation Data |            |
|----------------|--------------|----------------|------------|
| Sum	           | 13,830,300   | Sum	           | 743,664    |
| Median	       | 3,456,710.5  | Median	       | 111,001    |
| Geometric Mean | 3,457,532.90 | Geometric Mean | 122,097.68 |
| Largest	       | 3,477,079    | Largest	       | 484,310    |
| Smallest	     | 3,439,800    | Smallest	     | 37,352     |
| Range	         | 37,279       | Range	         | 446,958    |


### Separated Type Info

| Runtimes         | Deviations    |
|------------------|---------------|
| 2,427,858 / iter | (+/- 240,436) |
| 2,500,657 / iter | (+/- 59,630)  |
| 2,511,781 / iter | (+/- 135,379) |
| 2,513,021 / iter | (+/- 226,994) |

| Runtime Data   |              | Deviation Data |            |
|----------------|--------------|----------------|------------|
| Sum	           | 9,953,317    | Sum	           | 662,439    |
| Median	       | 2,506,219    | Median	       | 181,186.5  |
| Geometric Mean | 2,488,077.04 | Geometric Mean | 144,879.69 |
| Largest	       | 2,513,021    | Largest	       | 240,436    |
| Smallest	     | 2,427,858    | Smallest	     | 59,630     |
| Range	         | 85,163       | Range	         | 180,806    |


### Nan Tagged

| Runtimes         | Deviations    |
|------------------|---------------|
| 2,342,061 / iter | (+/- 124,897) |
| 2,401,524 / iter | (+/- 227,725) |
| 2,365,627 / iter | (+/- 124,049) |
| 2,296,237 / iter | (+/- 433,555) |

| Runtime Data   |              | Deviation Data |            |
|----------------|--------------|----------------|------------|
| Sum	           | 9,405,449    | Sum	           | 910,226    |
| Median	       | 2,353,844    | Median	       | 176,311    |
| Geometric Mean | 2,351,050.88 | Geometric Mean | 197,765.28 |
| Largest	       | 2,401,524    | Largest	       | 433,555    |
| Smallest	     | 2,296,237    | Smallest	     | 124,049    |
| Range	         | 105,287      | Range	         | 309,506    |




## Conclusions

First of all, its important to realise the fragility of benchmarks. In creating these, I had to be very careful to make all other factors the same, and often small and seemingly inconsequential changes would have drastic effects on performance, changing the rankings. So this, as with any benchmark, is only a rough comparison of one situation;not at all the final word on which is better in the general case.

Too, one must consider the specific "real-life" case this is modeling, and whether or not it is a common case for your VM. Here, we are testing these implementations for their speed at raw mathematics on floating point numbers; if that is not something your VM will be doing a lot of, this model won't tell you much and could likely be misleading.

All of that being said, we can take away some general insights here:

1. The packed union is almost certainly not worth the complexity tradeoff, as its performance is only **~2%** better than the naive implementation in the best case, and it has much larger deviations. This is most likely due to alignment to cache lines; while you can only fit 4 of the padded values, the lack of alignment of the unpadded version means you end up with values straddling cache line boundaries.

2. All alternatives have higher deviation than the naive implementation, due to their reliance on various tricks. These tricks all have tradeoffs that can vary in impact depending on where values end up in memory and other factors.

3. Nan tagging, as the conventional wisdom says, is fastest. However, due to its low supported variant limit of 8, you may have to store additional type information across a boundary, pessimizing performance of other value types. This does give you a **~35%** improvement over the naive case for numbers though, so if you are doing a lot of math, its probably worth it.

4. Separated type info is around **~5%** slower than nan-tagging, but here the type info is directly parallel and easily accessible. This is applicable for values on the stack, but heap allocated values will most likely have additional overhead for type checking due to the lack of a convenient parallel storage structure.

I would say, if you're crunching lots of numbers, go with nan tagging if you can, and parallel storage as a backup. Otherwise, its probably best to just go with the naive implementation. I would like to reiterate though, that other factors can have much larger impact than the value type chosen. Just using Rust's iterators rearranges the performance chart from what you see here!
