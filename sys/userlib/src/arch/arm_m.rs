// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! User application architecture support stubs for ARM.
//!
//! See the note on syscall stubs at the top of the userlib module for
//! rationale.

use crate::*;

/// This is the entry point for the kernel. Its job is to set up our memory
/// before jumping to user-defined `main`.
#[doc(hidden)]
#[no_mangle]
#[link_section = ".text.start"]
#[naked]
pub unsafe extern "C" fn _start() -> ! {
    // Provided by the user program:
    extern "Rust" {
        fn main() -> !;
    }

    asm!("
        @ Copy data initialization image into data section.
        @ Note: this assumes that both source and destination are 32-bit
        @ aligned and padded to 4-byte boundary.

        movw r0, #:lower16:__edata  @ upper bound in r0
        movt r0, #:upper16:__edata

        movw r1, #:lower16:__sidata @ source in r1
        movt r1, #:upper16:__sidata

        movw r2, #:lower16:__sdata  @ dest in r2
        movt r2, #:upper16:__sdata

        b 1f                        @ check for zero-sized data

    2:  ldr r3, [r1], #4            @ read and advance source
        str r3, [r2], #4            @ write and advance dest

    1:  cmp r2, r0                  @ has dest reached the upper bound?
        bne 2b                      @ if not, repeat

        @ Zero BSS section.

        movw r0, #:lower16:__ebss   @ upper bound in r0
        movt r0, #:upper16:__ebss

        movw r1, #:lower16:__sbss   @ base in r1
        movt r1, #:upper16:__sbss

        movs r2, #0                 @ materialize a zero

        b 1f                        @ check for zero-sized BSS

    2:  str r2, [r1], #4            @ zero one word and advance

    1:  cmp r1, r0                  @ has base reached bound?
        bne 2b                      @ if not, repeat

        @ Be extra careful to ensure that those side effects are
        @ visible to the user program.

        dsb         @ complete all writes
        isb         @ and flush the pipeline

        @ Now, to the user entry point. We call it in case it
        @ returns. (It's not supposed to.) We reference it through
        @ a sym operand because it's a Rust func and may be mangled.
        bl {main}

        @ The noreturn option below will automatically generate an
        @ undefined instruction trap past this point, should main
        @ return.
        ",
        main = sym main,
        options(noreturn),
    )
}

/// Core implementation of the REFRESH_TASK_ID syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_refresh_task_id_stub(_tid: u32) -> u32 {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need (plus one to
        @ maintain alignment); this means the pop sequence at the end needs to
        @ match!
        push {{r4, r5, r11, lr}}

        @ Move register arguments into place.
        mov r4, r0
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move result into place.
        mov r0, r4

        @ Restore the registers we used and return.
        pop {{r4, r5, r11, pc}}
        ",
        sysnum = const Sysnum::RefreshTaskId as u32,
        options(noreturn),
    )
}

/// Core implementation of the SEND syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_send_stub(
    _args: &mut SendArgs<'_>,
) -> RcLen {
    asm!("
        @ Spill the registers we're about to use to pass stuff.
        push {{r4-r11}}
        @ Load in args from the struct.
        ldm r0, {{r4-r10}}
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move the two results back into their return positions.
        mov r0, r4
        mov r1, r5
        @ Restore the registers we used.
        pop {{r4-r11}}
        @ Fin.
        bx lr
        ",
        sysnum = const Sysnum::Send as u32,
        options(noreturn),
    )
}

/// Core implementation of the RECV syscall.
#[naked]
#[must_use]
pub(crate) unsafe extern "C" fn sys_recv_stub(
    _buffer_ptr: *mut u8,
    _buffer_len: usize,
    _notification_mask: u32,
    _specific_sender: u32,
    _out: *mut crate::RawRecvMessage,
) -> u32 {
    asm!("
        @ Spill the registers we're about to use to pass stuff.
        push {{r4-r11}}
        @ Move register arguments into their proper positions.
        mov r4, r0
        mov r5, r1
        mov r6, r2
        mov r7, r3
        @ Read output buffer pointer from stack into a register that
        @ is preserved during our syscall. Since we just pushed a
        @ bunch of stuff, we need to read *past* it.
        ldr r3, [sp, #(8 * 4)]
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move status flag (only used for closed receive) into return
        @ position
        mov r0, r4
        @ Write all the results out into the raw output buffer.
        stm r3, {{r5-r9}}
        @ Restore the registers we used.
        pop {{r4-r11}}
        @ Fin.
        bx lr
        ",
        sysnum = const Sysnum::Recv as u32,
        options(noreturn),
    )
}

/// Core implementation of the REPLY syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_reply_stub(
    _peer: u32,
    _code: u32,
    _message_ptr: *const u8,
    _message_len: usize,
) {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match! (Why are we pushing LR? Because
        @ the ABI requires us to maintain 8-byte stack alignment, so we must
        @ push registers in pairs.)
        push {{r4-r7, r11, lr}}

        @ Move register arguments into place.
        mov r4, r0
        mov r5, r1
        mov r6, r2
        mov r7, r3
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ This call has no results.

        @ Restore the registers we used and return.
        pop {{r4-r7, r11, pc}}
        ",
        sysnum = const Sysnum::Reply as u32,
        options(noreturn),
    )
}

/// Core implementation of the SET_TIMER syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_set_timer_stub(
    _set_timer: u32,
    _deadline_lo: u32,
    _deadline_hi: u32,
    _notification: u32,
) {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match! (Why are we pushing LR? Because
        @ the ABI requires us to maintain 8-byte stack alignment, so we must
        @ push registers in pairs.)
        push {{r4-r7, r11, lr}}

        @ Move register arguments into place.
        mov r4, r0
        mov r5, r1
        mov r6, r2
        mov r7, r3
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ This call has no results.

        @ Restore the registers we used and return.
        pop {{r4-r7, r11, pc}}
        ",
        sysnum = const Sysnum::SetTimer as u32,
        options(noreturn),
    )
}

/// Core implementation of the BORROW_READ syscall.
///
/// See the note on syscall stubs at the top of this module for rationale.
#[naked]
pub(crate) unsafe extern "C" fn sys_borrow_read_stub(
    _args: *mut BorrowReadArgs,
) -> RcLen {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match!
        push {{r4-r8, r11}}

        @ Move register arguments into place.
        ldm r0, {{r4-r8}}
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move the results into place.
        mov r0, r4
        mov r1, r5

        @ Restore the registers we used and return.
        pop {{r4-r8, r11}}
        bx lr
        ",
        sysnum = const Sysnum::BorrowRead as u32,
        options(noreturn),
    )
}

/// Core implementation of the BORROW_WRITE syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_borrow_write_stub(
    _args: *mut BorrowWriteArgs,
) -> RcLen {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match!
        push {{r4-r8, r11}}

        @ Move register arguments into place.
        ldm r0, {{r4-r8}}
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move the results into place.
        mov r0, r4
        mov r1, r5

        @ Restore the registers we used and return.
        pop {{r4-r8, r11}}
        bx lr
        ",
        sysnum = const Sysnum::BorrowWrite as u32,
        options(noreturn),
    )
}

/// Core implementation of the BORROW_INFO syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_borrow_info_stub(
    _lender: u32,
    _index: usize,
    _out: *mut RawBorrowInfo,
) {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match!
        push {{r4-r6, r11}}

        @ Move register arguments into place.
        mov r4, r0
        mov r5, r1
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move the results into place.
        stm r2, {{r4-r6}}

        @ Restore the registers we used and return.
        pop {{r4-r6, r11}}
        bx lr
        ",
        sysnum = const Sysnum::BorrowInfo as u32,
        options(noreturn),
    )
}

/// Core implementation of the IRQ_CONTROL syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_irq_control_stub(_mask: u32, _enable: u32) {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match!
        push {{r4, r5, r11, lr}}

        @ Move register arguments into place.
        mov r4, r0
        mov r5, r1
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ This call returns no results.

        @ Restore the registers we used and return.
        pop {{r4, r5, r11, pc}}
        ",
        sysnum = const Sysnum::IrqControl as u32,
        options(noreturn),
    )
}

/// Core implementation of the PANIC syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_panic_stub(
    _msg: *const u8,
    _len: usize,
) -> ! {
    asm!("
        @ We're not going to return, so technically speaking we don't need to
        @ save registers. However, we save them anyway, so that we can reconstruct
        @ the state that led to the panic.
        push {{r4, r5, r11, lr}}

        @ Move register arguments into place.
        mov r4, r0
        mov r5, r1
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ This really shouldn't return. Ensure this:
        udf #0xad
        ",
        sysnum = const Sysnum::Panic as u32,
        options(noreturn),
    )
}

/// Core implementation of the GET_TIMER syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_get_timer_stub(_out: *mut RawTimerState) {
    asm!("
        @ Spill the registers we're about to use to pass stuff.
        push {{r4-r11}}
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Write all the results out into the raw output buffer.
        stm r0, {{r4-r9}}
        @ Restore the registers we used.
        pop {{r4-r11}}
        @ Fin.
        bx lr
        ",
        sysnum = const Sysnum::GetTimer as u32,
        options(noreturn),
    )
}

/// Core implementation of the POST syscall.
#[naked]
pub(crate) unsafe extern "C" fn sys_post_stub(_tid: u32, _mask: u32) -> u32 {
    asm!("
        @ Spill the registers we're about to use to pass stuff. Note that we're
        @ being clever and pushing only the registers we need; this means the
        @ pop sequence at the end needs to match!
        push {{r4, r5, r11, lr}}

        @ Move register arguments into place.
        mov r4, r0
        mov r5, r1
        @ Load the constant syscall number.
        mov r11, {sysnum}

        @ To the kernel!
        svc #0

        @ Move result into place.
        mov r0, r4

        @ Restore the registers we used and return.
        pop {{r4, r5, r11, pc}}
        ",
        sysnum = const Sysnum::Post as u32,
        options(noreturn),
    )
}
