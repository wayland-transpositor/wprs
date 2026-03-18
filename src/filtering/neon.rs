// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Filtering pipeline for aarch64 using NEON structure load/store.
//!
//! The forward transform (AoS→SoA, subtract_green, running_difference) and
//! its inverse (prefix_sum, add_green, SoA→AoS) are fused within each
//! 32-pixel block so channel data stays register-resident across all stages.
//! The transpose step uses `vld4q_u8`/`vst4q_u8` — hardware four-channel
//! deinterleave/interleave.
//!
//! # `unsafe` and `#[target_feature]`
//!
//! NEON is architecturally mandatory on AArch64 (ARMv8-A and later), but
//! `std::arch::aarch64` intrinsics carry `#[target_feature(enable = "neon")]`.
//! Per RFC 2396 (stabilized in Rust 1.86), calling such functions requires
//! `unsafe` unless the *caller* also carries a matching `#[target_feature]`
//! annotation — the compilation target's default features alone do not
//! suffice. The `unsafe` blocks in this module exist solely for this reason
//! when the enclosed operations are register-to-register.

use std::arch::aarch64::*;

// --- 16-byte primitives ---

/// Running difference: `out[i] = in[i] - in[i-1]`, carry in NEON domain.
#[inline(always)]
fn running_difference_16(block: uint8x16_t, prev: uint8x16_t) -> (uint8x16_t, uint8x16_t) {
    // SAFETY: register-to-register NEON; see module doc on #[target_feature].
    unsafe {
        let shifted = vextq_u8::<15>(prev, block);
        (vsubq_u8(block, shifted), vdupq_laneq_u8::<15>(block))
    }
}

/// Inclusive prefix sum: Hillis-Steele doubling, 4 steps for 16 lanes.
#[inline(always)]
fn prefix_sum_16(mut block: uint8x16_t) -> uint8x16_t {
    // SAFETY: register-to-register NEON; see module doc on #[target_feature].
    unsafe {
        let zero = vdupq_n_u8(0);
        block = vaddq_u8(block, vextq_u8::<15>(zero, block));
        block = vaddq_u8(block, vextq_u8::<14>(zero, block));
        block = vaddq_u8(block, vextq_u8::<12>(zero, block));
        block = vaddq_u8(block, vextq_u8::<8>(zero, block));
        block
    }
}

/// Chain local prefix sums across blocks via broadcast carry.
#[inline(always)]
fn accumulate_16(block: uint8x16_t, carry: uint8x16_t) -> (uint8x16_t, uint8x16_t) {
    // SAFETY: register-to-register NEON; see module doc on #[target_feature].
    unsafe {
        let block_last = vdupq_laneq_u8::<15>(block);
        (vaddq_u8(block, carry), vaddq_u8(carry, block_last))
    }
}

// --- 32-pixel fused blocks ---

/// Forward: deinterleave → subtract_green → running_difference, 32 pixels.
///
/// Carry state stays in the NEON register file throughout; the forward and
/// inverse transforms are now symmetric in their carry representation.
#[inline]
fn aos_to_soa_u8_32x4(
    input: *const u8,
    out0: &mut [u8; 32],
    out1: &mut [u8; 32],
    out2: &mut [u8; 32],
    out3: &mut [u8; 32],
    prev0: uint8x16_t,
    prev1: uint8x16_t,
    prev2: uint8x16_t,
    prev3: uint8x16_t,
) -> (uint8x16_t, uint8x16_t, uint8x16_t, uint8x16_t) {
    // SAFETY: `input` points to 128 bytes (32 × Vec4u8) from array_chunks::<32>.
    // vld4q_u8 reads 64 bytes; vst1q_u8 writes 16 bytes per half.
    unsafe {
        let lo = vld4q_u8(input);
        let hi = vld4q_u8(input.add(64));

        let (ch0_lo, ch0_hi) = (lo.0, hi.0);
        let (ch1_lo, ch1_hi) = (lo.1, hi.1);
        let (ch2_lo, ch2_hi) = (lo.2, hi.2);
        let (ch3_lo, ch3_hi) = (lo.3, hi.3);

        // subtract_green: B -= G, R -= G
        let ch0_lo = vsubq_u8(ch0_lo, ch1_lo);
        let ch0_hi = vsubq_u8(ch0_hi, ch1_hi);
        let ch2_lo = vsubq_u8(ch2_lo, ch1_lo);
        let ch2_hi = vsubq_u8(ch2_hi, ch1_hi);

        // running_difference per channel
        let (d0_lo, mid0) = running_difference_16(ch0_lo, prev0);
        let (d0_hi, next0) = running_difference_16(ch0_hi, mid0);
        let (d1_lo, mid1) = running_difference_16(ch1_lo, prev1);
        let (d1_hi, next1) = running_difference_16(ch1_hi, mid1);
        let (d2_lo, mid2) = running_difference_16(ch2_lo, prev2);
        let (d2_hi, next2) = running_difference_16(ch2_hi, mid2);
        let (d3_lo, mid3) = running_difference_16(ch3_lo, prev3);
        let (d3_hi, next3) = running_difference_16(ch3_hi, mid3);

        vst1q_u8(out0.as_mut_ptr(), d0_lo);
        vst1q_u8(out0.as_mut_ptr().add(16), d0_hi);
        vst1q_u8(out1.as_mut_ptr(), d1_lo);
        vst1q_u8(out1.as_mut_ptr().add(16), d1_hi);
        vst1q_u8(out2.as_mut_ptr(), d2_lo);
        vst1q_u8(out2.as_mut_ptr().add(16), d2_hi);
        vst1q_u8(out3.as_mut_ptr(), d3_lo);
        vst1q_u8(out3.as_mut_ptr().add(16), d3_hi);

        (next0, next1, next2, next3)
    }
}

/// Inverse: prefix_sum → add_green → interleave, 32 pixels.
#[inline]
fn soa_to_aos_u8_32x4(
    input0: &[u8; 32],
    input1: &[u8; 32],
    input2: &[u8; 32],
    input3: &[u8; 32],
    out: &mut [Vec4u8; 32],
    prev0: uint8x16_t,
    prev1: uint8x16_t,
    prev2: uint8x16_t,
    prev3: uint8x16_t,
) -> (uint8x16_t, uint8x16_t, uint8x16_t, uint8x16_t) {
    // SAFETY: vld1q_u8 reads 16 bytes per half; vst4q_u8 writes 64 bytes × 2.
    unsafe {
        let ch0_lo = vld1q_u8(input0.as_ptr());
        let ch0_hi = vld1q_u8(input0.as_ptr().add(16));
        let ch1_lo = vld1q_u8(input1.as_ptr());
        let ch1_hi = vld1q_u8(input1.as_ptr().add(16));
        let ch2_lo = vld1q_u8(input2.as_ptr());
        let ch2_hi = vld1q_u8(input2.as_ptr().add(16));
        let ch3_lo = vld1q_u8(input3.as_ptr());
        let ch3_hi = vld1q_u8(input3.as_ptr().add(16));

        // prefix_sum per channel (inverse of running_difference)
        let ch0_lo = prefix_sum_16(ch0_lo);
        let (ch0_lo, prev0) = accumulate_16(ch0_lo, prev0);
        let ch0_hi = prefix_sum_16(ch0_hi);
        let (ch0_hi, prev0) = accumulate_16(ch0_hi, prev0);

        let ch1_lo = prefix_sum_16(ch1_lo);
        let (ch1_lo, prev1) = accumulate_16(ch1_lo, prev1);
        let ch1_hi = prefix_sum_16(ch1_hi);
        let (ch1_hi, prev1) = accumulate_16(ch1_hi, prev1);

        let ch2_lo = prefix_sum_16(ch2_lo);
        let (ch2_lo, prev2) = accumulate_16(ch2_lo, prev2);
        let ch2_hi = prefix_sum_16(ch2_hi);
        let (ch2_hi, prev2) = accumulate_16(ch2_hi, prev2);

        let ch3_lo = prefix_sum_16(ch3_lo);
        let (ch3_lo, prev3) = accumulate_16(ch3_lo, prev3);
        let ch3_hi = prefix_sum_16(ch3_hi);
        let (ch3_hi, prev3) = accumulate_16(ch3_hi, prev3);

        // add_green (inverse VP8L): B += G, R += G
        let ch0_lo = vaddq_u8(ch0_lo, ch1_lo);
        let ch0_hi = vaddq_u8(ch0_hi, ch1_hi);
        let ch2_lo = vaddq_u8(ch2_lo, ch1_lo);
        let ch2_hi = vaddq_u8(ch2_hi, ch1_hi);

        // interleave
        let out_ptr = out.as_mut_ptr().cast::<u8>();
        vst4q_u8(out_ptr, uint8x16x4_t(ch0_lo, ch1_lo, ch2_lo, ch3_lo));
        vst4q_u8(
            out_ptr.add(64),
            uint8x16x4_t(ch0_hi, ch1_hi, ch2_hi, ch3_hi),
        );

        (prev0, prev1, prev2, prev3)
    }
}

// --- kernel trait impl ---

use crate::buffer_pointer::KnownSizeBufferPointer;
use crate::filtering::FilterKernel;
use crate::vec4u8::Vec4u8;

type Carry4 = (uint8x16_t, uint8x16_t, uint8x16_t, uint8x16_t);

pub(super) struct Kernel;

impl FilterKernel for Kernel {
    type ForwardCarry = uint8x16_t;
    type InverseCarry = uint8x16_t;

    fn zero_forward() -> Carry4 {
        // SAFETY: register broadcast; see module doc on #[target_feature].
        let z = unsafe { vdupq_n_u8(0) };
        (z, z, z, z)
    }

    fn zero_inverse() -> Carry4 {
        Self::zero_forward()
    }

    fn forward_block(
        input: KnownSizeBufferPointer<Vec4u8, 32>,
        out0: &mut [u8; 32],
        out1: &mut [u8; 32],
        out2: &mut [u8; 32],
        out3: &mut [u8; 32],
        (p0, p1, p2, p3): Carry4,
    ) -> Carry4 {
        aos_to_soa_u8_32x4(
            input.ptr().cast::<u8>(),
            out0,
            out1,
            out2,
            out3,
            p0,
            p1,
            p2,
            p3,
        )
    }

    fn inverse_block(
        in0: &[u8; 32],
        in1: &[u8; 32],
        in2: &[u8; 32],
        in3: &[u8; 32],
        out: &mut [Vec4u8; 32],
        (p0, p1, p2, p3): Carry4,
    ) -> Carry4 {
        soa_to_aos_u8_32x4(in0, in1, in2, in3, out, p0, p1, p2, p3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: extract 16 bytes from a NEON register.
    fn to_array(v: uint8x16_t) -> [u8; 16] {
        let mut out = [0u8; 16];
        // SAFETY: vst1q_u8 writes 16 bytes to a 16-byte array.
        unsafe { vst1q_u8(out.as_mut_ptr(), v) };
        out
    }

    /// Helper: load 16 bytes into a NEON register.
    fn from_array(a: &[u8; 16]) -> uint8x16_t {
        // SAFETY: vld1q_u8 reads 16 bytes from a 16-byte array.
        unsafe { vld1q_u8(a.as_ptr()) }
    }

    #[test]
    fn test_running_difference_16() {
        let input: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        // SAFETY: register broadcast; see module doc on #[target_feature].
        let zero = unsafe { vdupq_n_u8(0) };
        let (result, carry) = running_difference_16(from_array(&input), zero);
        assert_eq!(
            to_array(result),
            [0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
        );
        assert_eq!(to_array(carry), [15; 16]);
    }

    #[test]
    fn test_running_difference_16_with_carry() {
        let input: [u8; 16] = [
            10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
        ];
        // SAFETY: register broadcast; see module doc on #[target_feature].
        let carry_in = unsafe { vdupq_n_u8(9) };
        let (result, carry) = running_difference_16(from_array(&input), carry_in);
        assert_eq!(
            to_array(result),
            [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]
        );
        assert_eq!(to_array(carry), [25; 16]);
    }

    #[test]
    fn test_prefix_sum_16() {
        let input: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let result = prefix_sum_16(from_array(&input));
        // Inclusive prefix sum: [0, 0+1, 0+1+2, ...] = [0, 1, 3, 6, 10, 15, 21, 28, ...]
        assert_eq!(
            to_array(result),
            [0, 1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 66, 78, 91, 105, 120]
        );
    }

    #[test]
    fn test_prefix_sum_roundtrip() {
        // prefix_sum is left inverse of running_difference.
        let input: [u8; 16] = [
            42, 17, 255, 0, 1, 128, 99, 200, 3, 7, 11, 13, 50, 60, 70, 80,
        ];
        // SAFETY: register broadcast; see module doc on #[target_feature].
        let zero = unsafe { vdupq_n_u8(0) };
        let (diff, carry) = running_difference_16(from_array(&input), zero);
        let recovered = prefix_sum_16(diff);
        assert_eq!(to_array(recovered), input);
        assert_eq!(to_array(carry), [80; 16]);
    }

    #[test]
    fn test_accumulate_16() {
        let block: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        // SAFETY: register broadcast; see module doc on #[target_feature].
        let carry = unsafe { vdupq_n_u8(100) };
        let (result, new_carry) = accumulate_16(from_array(&block), carry);
        // Each element += 100.
        assert_eq!(
            to_array(result),
            [
                101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116
            ]
        );
        // New carry = old carry + last element of block = 100 + 16 = 116.
        assert_eq!(to_array(new_carry), [116; 16]);
    }
}
