// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![no_std]
#![no_main]

// Make sure we actually link in userlib, despite not using any of it explicitly
// - we need it for our _start routine.
extern crate userlib;

#[cfg(target_arch = "riscv32")]
use userlib::*;

#[export_name = "main"]
fn main() -> ! {
    loop {
        // Wait For Interrupt to pause the processor until an ISR arrives,
        // which could wake some higher-priority task.
        #[cfg(target_arch = "arm")]
        cortex_m::asm::wfi();

        // RISC-V has wfi, but unfortunately it is an illegal instruction if
        // called from User mode, so instead we spin on a timer call.
        #[cfg(target_arch = "riscv32")]
        while sys_get_timer().now > 0 {}
    }
}
