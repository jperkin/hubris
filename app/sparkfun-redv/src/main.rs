// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![no_std]
#![no_main]

extern crate panic_halt;
extern crate riscv_rt;

use riscv_rt::entry;

use kern::app::App;

extern "C" {
    static hubris_app_table: App;
    static mut _sheap: u8;
    static __eheap: u8;
}

#[entry]
fn main() -> ! {
    const CYCLES_PER_MS: u32 = 8_000;

    unsafe {
        let heap_size =
            (&__eheap as *const _ as usize) - (&_sheap as *const _ as usize);
        kern::startup::start_kernel(
            &hubris_app_table,
            (&mut _sheap) as *mut _,
            heap_size,
            CYCLES_PER_MS,
        )
    }
}
