// Copyright 2024 Google LLC
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

use wprs_common::buffer_pointer::BufferPointer;

fn works_when_life_of_ptr_is_long_enough() {
    let a = [0u32; 4];
    let a_ptr = a.as_ptr();
    let _bufptr = unsafe { BufferPointer::new(&a_ptr, a.len()) };
}

fn does_not_compile_when_lifetime_of_ptr_too_short() {
    let _bufptr = {
        let a = [0u32; 4];
        let a_ptr = a.as_ptr();
        unsafe { BufferPointer::new(&a_ptr, a.len()) }
    };
}

fn cast_does_not_extend_lifetime() {
    let _bufptr: BufferPointer<*const u8> = unsafe {
        let a = [0u32; 4];
        let a_ptr = a.as_ptr();
        let bufptr = BufferPointer::new(&a_ptr, a.len());
        bufptr.cast()
    };
}

fn split_at_does_not_extend_lifetime() {
    let (_bufptr1, _bufptr2) = unsafe {
        let a = [0u32; 4];
        let a_ptr = a.as_ptr();
        let bufptr = BufferPointer::new(&a_ptr, a.len());
        bufptr.split_at(2)
    };
}

fn main() {
    works_when_life_of_ptr_is_long_enough();
    does_not_compile_when_lifetime_of_ptr_too_short();
    cast_does_not_extend_lifetime();
    split_at_does_not_extend_lifetime();
}
