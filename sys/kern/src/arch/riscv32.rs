// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Architecture support for RISC-V.
//!
//! Currently only the Freedom E310 core as found on the HiFive RevB and
//! Sparkfun RED-V boards is supported, but adding support for other cores
//! should be straight-forward.
//!
//! As the FE310 core does not support it there is no Supervisor mode support,
//! the kernel runs exclusively in Machine mode with tasks running in User
//! mode.
//!
//! Interrupts (other than the Machine Timer used to advance the kernel
//! timestamp) are not yet supported.

use core::ptr::NonNull;

use zerocopy::FromBytes;

use crate::app;
use crate::task;
use crate::time::Timestamp;
use abi::{FaultInfo, FaultSource};

extern crate riscv_rt;

use riscv::register;
use riscv::register::mcause::{Exception, Interrupt, Trap};
use riscv::register::mstatus::MPP;

/// Log things from kernel context. This macro is made visible to the rest of
/// the kernel by a chain of `#[macro_use]` attributes, but its implementation
/// is very architecture-specific at the moment.
///
/// On RISC-V we are limited to semihosting, despite its shortcomings.
///
/// In the future, we will likely want to add at least one more mechanism for
/// logging (one that can be presumably be made neutral with respect to
/// architecure), whereby kernel logs can be produced somewhere (e.g., a ring
/// buffer) from which they can be consumed by some entity for shipping
/// elsewhere.
///
macro_rules! klog {
    ($s:expr) => { let _ = riscv_semihosting::hprintln!($s); };
    ($s:expr, $($tt:tt)*) => { let _ = riscv_semihosting::hprintln!($s, $($tt)*); };
}
#[allow(unused_macros)]
macro_rules! kdbg {
    ($s:expr) => { let _ = riscv_semihosting::dbg!($s); };
    ($s:expr, $($tt:tt)*) => { let _ = riscv_semihosting::dbg!($s, $($tt)*); };
}

macro_rules! uassert {
    ($cond : expr) => {
        if !$cond {
            panic!("Assertion failed!");
        }
    };
}

macro_rules! uassert_eq {
    ($cond1 : expr, $cond2 : expr) => {
        if !($cond1 == $cond2) {
            panic!("Assertion failed!");
        }
    };
}

/// On RISC-V we use a global to record the task table position and extent.
#[no_mangle]
static mut TASK_TABLE_BASE: Option<NonNull<task::Task>> = None;
#[no_mangle]
static mut TASK_TABLE_SIZE: usize = 0;

/// On RISC-V we use a global to record the interrupt table position and extent.
#[no_mangle]
static mut IRQ_TABLE_BASE: Option<NonNull<abi::Interrupt>> = None;
#[no_mangle]
static mut IRQ_TABLE_SIZE: usize = 0;

/// On RISC-V we use a global to record the current task pointer.  It may be
/// possible to use the mscratch register instead.
#[no_mangle]
static mut CURRENT_TASK_PTR: Option<NonNull<task::Task>> = None;

/// To allow our clock frequency to be easily determined from a debugger, we
/// store it in memory.
#[no_mangle]
static mut CLOCK_FREQ_KHZ: u32 = 0;

/// RISC-V volatile registers that must be saved across context switches.
#[repr(C)]
#[derive(Clone, Debug, Default, FromBytes)]
pub struct SavedState {
    // NOTE: the following fields must be kept contiguous!
    ra: u32,
    sp: u32,
    gp: u32,
    tp: u32,
    t0: u32,
    t1: u32,
    t2: u32,
    s0: u32,
    s1: u32,
    a0: u32,
    a1: u32,
    a2: u32,
    a3: u32,
    a4: u32,
    a5: u32,
    a6: u32,
    a7: u32,
    s2: u32,
    s3: u32,
    s4: u32,
    s5: u32,
    s6: u32,
    s7: u32,
    s8: u32,
    s9: u32,
    s10: u32,
    s11: u32,
    t3: u32,
    t4: u32,
    t5: u32,
    t6: u32,
    // Additional save value for task program counter
    pc: u32,
    // NOTE: the above fields must be kept contiguous!
}

/// Map the volatile registers to (architecture-independent) syscall argument
/// and return slots.
impl task::ArchState for SavedState {
    fn stack_pointer(&self) -> u32 {
        self.sp
    }

    /// Reads syscall argument register 0.
    fn arg0(&self) -> u32 {
        self.a0
    }
    fn arg1(&self) -> u32 {
        self.a1
    }
    fn arg2(&self) -> u32 {
        self.a2
    }
    fn arg3(&self) -> u32 {
        self.a3
    }
    fn arg4(&self) -> u32 {
        self.a4
    }
    fn arg5(&self) -> u32 {
        self.a5
    }
    fn arg6(&self) -> u32 {
        self.a6
    }

    fn syscall_descriptor(&self) -> u32 {
        self.a7
    }

    /// Writes syscall return argument 0.
    fn ret0(&mut self, x: u32) {
        self.a0 = x
    }
    fn ret1(&mut self, x: u32) {
        self.a1 = x
    }
    fn ret2(&mut self, x: u32) {
        self.a2 = x
    }
    fn ret3(&mut self, x: u32) {
        self.a3 = x
    }
    fn ret4(&mut self, x: u32) {
        self.a4 = x
    }
    fn ret5(&mut self, x: u32) {
        self.a5 = x
    }
}

/// Records `tasks` as the system-wide task table.
///
/// If a task table has already been set, panics.
///
/// # Safety
///
/// This stashes a copy of `tasks` without revoking your right to access it,
/// which is a potential aliasing violation if you call `with_task_table`. So
/// don't do that. The normal kernel entry sequences avoid this issue.
pub unsafe fn set_task_table(tasks: &mut [task::Task]) {
    let prev_task_table = core::mem::replace(
        &mut TASK_TABLE_BASE,
        Some(NonNull::from(&mut tasks[0])),
    );
    // Catch double-uses of this function.
    uassert_eq!(prev_task_table, None);
    // Record length as well.
    TASK_TABLE_SIZE = tasks.len();
}

pub unsafe fn set_irq_table(irqs: &[abi::Interrupt]) {
    let prev_table = core::mem::replace(
        &mut IRQ_TABLE_BASE,
        Some(NonNull::new_unchecked(irqs.as_ptr() as *mut abi::Interrupt)),
    );
    // Catch double-uses of this function.
    uassert_eq!(prev_table, None);
    // Record length as well.
    IRQ_TABLE_SIZE = irqs.len();
}

pub fn reinitialize(task: &mut task::Task) {
    *task.save_mut() = SavedState::default();

    // Set the initial stack pointer, ensuring 16-byte stack alignment as per
    // the RISC-V calling convention.
    task.save_mut().sp = task.descriptor().initial_stack;
    uassert!(task.save().sp & 0xF == 0);

    // Set the initial program counter
    task.save_mut().pc = task.descriptor().entry_point;
}

#[allow(unused_variables)]
pub fn apply_memory_protection(task: &task::Task) {
    for (i, region) in task.region_table().iter().enumerate() {
        let mut pmpcfg: usize = 0;
        if region.attributes.contains(app::RegionAttributes::READ) {
            pmpcfg |= 0b001;
        }
        if region.attributes.contains(app::RegionAttributes::WRITE) {
            pmpcfg |= 0b010;
        }
        if region.attributes.contains(app::RegionAttributes::EXECUTE) {
            pmpcfg |= 0b100;
        }
        // Configure NAPOT (naturally aligned power-of-2) regions
        pmpcfg |= 0b11_000;

        let mut pmpaddr: usize = 0;
        pmpaddr |= region.base as usize >> 2;
        pmpaddr |= (region.size - 1) as usize;

        match i {
            0 => {
                register::pmpaddr0::write(pmpaddr);
                register::pmpcfg0::write(
                    register::pmpcfg0::read().bits & 0xFFFF_FF00 | pmpcfg,
                );
            }
            1 => {
                register::pmpaddr1::write(pmpaddr);
                register::pmpcfg0::write(
                    register::pmpcfg0::read().bits & 0xFFFF_00FF
                        | (pmpcfg << 8),
                );
            }
            2 => {
                register::pmpaddr2::write(pmpaddr);
                register::pmpcfg0::write(
                    register::pmpcfg0::read().bits & 0xFF00_FFFF
                        | (pmpcfg << 16),
                );
            }
            3 => {
                register::pmpaddr3::write(pmpaddr);
                register::pmpcfg0::write(
                    register::pmpcfg0::read().bits & 0x00FF_FFFF
                        | (pmpcfg << 24),
                );
            }
            4 => {
                register::pmpaddr4::write(pmpaddr);
                register::pmpcfg1::write(
                    register::pmpcfg1::read().bits & 0xFFFF_FF00 | pmpcfg,
                );
            }
            5 => {
                register::pmpaddr5::write(pmpaddr);
                register::pmpcfg1::write(
                    register::pmpcfg1::read().bits & 0xFFFF_00FF
                        | (pmpcfg << 8),
                );
            }
            6 => {
                register::pmpaddr6::write(pmpaddr);
                register::pmpcfg1::write(
                    register::pmpcfg1::read().bits & 0xFF00_FFFF
                        | (pmpcfg << 16),
                );
            }
            7 => {
                register::pmpaddr7::write(pmpaddr);
                register::pmpcfg1::write(
                    register::pmpcfg1::read().bits & 0x00FF_FFFF
                        | (pmpcfg << 24),
                );
            }
            _ => {}
        };
    }
}

// Provide our own interrupt vector to handle save/restore of the task on
// entry, overwriting the symbol set up by riscv-rt.  The repr(align(4)) is
// necessary as the bottom bits are used to determine direct or vectored traps.
//
// We may want to switch to a vectored interrupt table at some point to improve
// performance.
#[naked]
#[no_mangle]
#[repr(align(4))]
#[link_section = ".trap.rust"]
#[export_name = "_start_trap"]
pub unsafe extern "C" fn _start_trap() {
    asm!(
        "
        #
        # Store full task status on entry, setting up a0 to point at our
        # current task so that it's passed into our exception handler.
        #
        csrw mscratch, a0
        la a0, CURRENT_TASK_PTR
        lw a0, (a0)
        sw ra,   0*4(a0)
        sw sp,   1*4(a0)
        sw gp,   2*4(a0)
        sw tp,   3*4(a0)
        sw t0,   4*4(a0)
        sw t1,   5*4(a0)
        sw t2,   6*4(a0)
        sw s0,   7*4(a0)
        sw s1,   8*4(a0)
        #sw a0,  9*4(a0)
        sw a1,  10*4(a0)
        sw a2,  11*4(a0)
        sw a3,  12*4(a0)
        sw a4,  13*4(a0)
        sw a5,  14*4(a0)
        sw a6,  15*4(a0)
        sw a7,  16*4(a0)
        sw s2,  17*4(a0)
        sw s3,  18*4(a0)
        sw s4,  19*4(a0)
        sw s5,  20*4(a0)
        sw s6,  21*4(a0)
        sw s7,  22*4(a0)
        sw s8,  23*4(a0)
        sw s9,  24*4(a0)
        sw s10, 25*4(a0)
        sw s11, 26*4(a0)
        sw t3,  27*4(a0)
        sw t4,  28*4(a0)
        sw t5,  29*4(a0)
        sw t6,  30*4(a0)
        csrr a1, mepc
        sw a1,  31*4(a0)    # store mepc for resume
        csrr a1, mscratch
        sw a1, 9*4(a0)      # store a0 itself

        #
        # Jump to our main rust handler
        #
        jal trap_handler

        #
        # On the way out we may have switched to a different task, load
        # everything in and resume (using t6 as it's resored last).
        #
        la t6, CURRENT_TASK_PTR
        lw t6, (t6)

        lw t5,  31*4(t6)     # restore mepc
        csrw mepc, t5

        lw ra,   0*4(t6)
        lw sp,   1*4(t6)
        lw gp,   2*4(t6)
        lw tp,   3*4(t6)
        lw t0,   4*4(t6)
        lw t1,   5*4(t6)
        lw t2,   6*4(t6)
        lw s0,   7*4(t6)
        lw s1,   8*4(t6)
        lw a0,   9*4(t6)
        lw a1,  10*4(t6)
        lw a2,  11*4(t6)
        lw a3,  12*4(t6)
        lw a4,  13*4(t6)
        lw a5,  14*4(t6)
        lw a6,  15*4(t6)
        lw a7,  16*4(t6)
        lw s2,  17*4(t6)
        lw s3,  18*4(t6)
        lw s4,  19*4(t6)
        lw s5,  20*4(t6)
        lw s6,  21*4(t6)
        lw s7,  22*4(t6)
        lw s8,  23*4(t6)
        lw s9,  24*4(t6)
        lw s10, 25*4(t6)
        lw s11, 26*4(t6)
        lw t3,  27*4(t6)
        lw t4,  28*4(t6)
        lw t5,  29*4(t6)
        lw t6,  30*4(t6)

        mret
        ",
        options(noreturn),
    );
}

//
// The Rust side of our trap handler after the task's registers have been
// saved to SavedState.
//
#[no_mangle]
fn trap_handler(task: &mut task::Task) {
    match register::mcause::read().cause() {
        //
        // Interrupts.  Only our periodic MachineTimer interrupt via mtime is
        // supported at present.
        //
        Trap::Interrupt(Interrupt::MachineTimer) => unsafe {
            let ticks = &mut TICKS;
            with_task_table(|tasks| safe_timer_handler(ticks, tasks));
        },
        //
        // System Calls.
        //
        Trap::Exception(Exception::UserEnvCall) => {
            unsafe {
                // Advance program counter past ecall instruction.
                task.save_mut().pc = register::mepc::read() as u32 + 4;
                asm!(
                    "
                    la a1, CURRENT_TASK_PTR
                    mv a0, a7               # arg0 = syscall number
                    lw a1, (a1)             # arg1 = task ptr
                    jal syscall_entry
                    ",
                );
            }
        }
        //
        // Exceptions.  Routed via the most appropriate FaultInfo.
        //
        Trap::Exception(Exception::IllegalInstruction) => unsafe {
            handle_fault(task, FaultInfo::IllegalInstruction);
        },
        Trap::Exception(Exception::LoadFault) => unsafe {
            handle_fault(
                task,
                FaultInfo::MemoryAccess {
                    address: Some(register::mtval::read() as u32),
                    source: FaultSource::User,
                },
            );
        },
        Trap::Exception(Exception::InstructionFault) => unsafe {
            handle_fault(task, FaultInfo::IllegalText);
        },
        _ => {
            klog!(
                "Unimplemented cause 0b{:b}",
                register::mcause::read().bits()
            );
        }
    }
}

#[no_mangle]
unsafe fn handle_fault(task: *mut task::Task, fault: FaultInfo) {
    with_task_table(|tasks| {
        let idx = (task as usize - tasks.as_ptr() as usize)
            / core::mem::size_of::<task::Task>();

        let next = match task::force_fault(tasks, idx, fault) {
            task::NextTask::Specific(i) => i,
            task::NextTask::Other => task::select(idx, tasks),
            task::NextTask::Same => idx,
        };

        if next == idx {
            panic!("attempt to return to Task #{} after fault", idx);
        }

        let next = &mut tasks[next];
        apply_memory_protection(next);
        set_current_task(next);
    });
}

// Timer handling.
//
// We currently only support single HART systems.  From reading elsewhere,
// additional harts have their own mtimecmp offset at 0x8 intervals from hart0.
//
// As per FE310-G002 Manual, section 9.1, the address of mtimecmp on
// our supported board is 0x0200_4000, which also matches qemu.
//
// On both RV32 and RV64 systems the mtime and mtimecmp memory-mapped registers
// are 64-bits wide.
//
const MTIMECMP: u64 = 0x0200_4000;
const MTIME: u64 = 0x0200_BFF8;

// Configure the timer.
//
// RISC-V Privileged Architecture Manual
// 3.2.1 Machine Timer Registers (mtime and mtimecmp)
//
// To keep things simple, especially on RV32 systems where we cannot atomically
// write to the mtime/mtimecmp memory-mapped registers as they are 64 bits
// wide, we only utilise the first 32-bits of each register, setting the
// high-order bits to 0 on startup, and restarting the low-order bits of mtime
// back to 0 on each interrupt.
//
#[no_mangle]
unsafe fn set_timer(tick_divisor: u32) {
    // Set high-order bits of mtime to zero.  We only call this function prior
    // to enabling interrupts so it should be safe.
    asm!("
        li {0}, {mtimecmp}  # load mtimecmp address
        li {1}, -1          # start with all low-order bits set
        sw {1}, 0({0})      # set low-order bits -1
        sw zero, 4({0})     # set high-order bits to 0
        sw {2}, 0({0})      # set low-order bits to tick_divisor

        li {0}, {mtime}     # load mtime address
        sw zero, 4({0})     # set high-order bits to 0
        sw zero, 0({0})     # set low-order bits back to 0
        ",
        out(reg) _,
        out(reg) _,
        in(reg) tick_divisor,
        mtime = const MTIME,
        mtimecmp = const MTIMECMP,
    );
}

fn safe_timer_handler(ticks: &mut u64, tasks: &mut [task::Task]) {
    // Advance the kernel's notion of time.
    // This increment is not expected to overflow in a working system, since it
    // would indicate that 2^64 ticks have passed, and ticks are expected to be
    // in the range of nanoseconds to milliseconds -- meaning over 500 years.
    // However, we do not use wrapping add here because, if we _do_ overflow due
    // to e.g. memory corruption, we'd rather panic and reboot than attempt to
    // limp forward.
    *ticks += 1;
    // Now, give up mutable access to *ticks so there's no chance of a
    // double-increment due to bugs below.
    let now = Timestamp::from(*ticks);
    drop(ticks);

    // Process any timers.
    let switch = task::process_timers(tasks, now);

    if switch != task::NextTask::Same {
        unsafe {
            with_task_table(|tasks| {
                let current = CURRENT_TASK_PTR
                    .expect("irq before kernel started?")
                    .as_ptr();
                let idx = (current as usize - tasks.as_ptr() as usize)
                    / core::mem::size_of::<task::Task>();

                let next = task::select(idx, tasks);
                let next = &mut tasks[next];
                apply_memory_protection(next);
                set_current_task(next);
            });
        }
    }

    unsafe {
        // Reset mtime back to 0.  In theory we could save an instruction on
        // RV32 here and only write the low-order bits, assuming that it has
        // been less than 12 seconds or so since our last interrupt(!), but
        // let's avoid any possibility of a nasty surprise.
        core::ptr::write_volatile(MTIME as *mut u64, 0);
    }
}

#[allow(unused_variables)]
pub fn start_first_task(tick_divisor: u32, task: &task::Task) -> ! {
    // Configure MPP to switch us to User mode on exit from Machine
    // mode (when we call "mret" below).
    unsafe {
        register::mstatus::set_mpp(MPP::User);
    }

    // Write the initial task program counter.
    register::mepc::write(task.save().pc as *const usize as usize);

    // Configure the timer
    unsafe {
        CLOCK_FREQ_KHZ = tick_divisor;

        // Reset mtime back to 0, set mtimecmp to chosen timer
        set_timer(tick_divisor - 1);

        // Machine timer interrupt enable
        register::mie::set_mtimer();

        // Global machine interrupt enable
        register::mstatus::set_mie();
    }

    // Load first task pointer, set its initial stack pointer, and exit out
    // of machine mode, launching the task.
    unsafe {
        CURRENT_TASK_PTR = Some(NonNull::from(task));
        asm!("
            lw sp, ({sp})
            mret",
            sp = in(reg) &task.save().sp,
            options(noreturn)
        );
    }
}

/// Manufacture a mutable/exclusive reference to the task table from thin air
/// and hand it to `body`. This bypasses borrow checking and should only be used
/// at kernel entry points, then passed around.
///
/// Because the lifetime of the reference passed into `body` is anonymous, the
/// reference can't easily be stored, which is deliberate.
///
/// # Safety
///
/// You can use this safely at kernel entry points, exactly once, to create a
/// reference to the task table.
pub unsafe fn with_task_table<R>(
    body: impl FnOnce(&mut [task::Task]) -> R,
) -> R {
    let tasks = core::slice::from_raw_parts_mut(
        TASK_TABLE_BASE.expect("kernel not started").as_mut(),
        TASK_TABLE_SIZE,
    );
    body(tasks)
}

/// Manufacture a shared reference to the interrupt action table from thin air
/// and hand it to `body`. This bypasses borrow checking and should only be used
/// at kernel entry points, then passed around.
///
/// Because the lifetime of the reference passed into `body` is anonymous, the
/// reference can't easily be stored, which is deliberate.
pub fn with_irq_table<R>(body: impl FnOnce(&[abi::Interrupt]) -> R) -> R {
    // Safety: as long as a legit pointer was stored in IRQ_TABLE_BASE, or no
    // pointer has been stored, we can do this safely.
    let table = unsafe {
        core::slice::from_raw_parts(
            IRQ_TABLE_BASE.expect("kernel not started").as_ptr(),
            IRQ_TABLE_SIZE,
        )
    };
    body(table)
}

/// Records the address of `task` as the current user task.
///
/// # Safety
///
/// This records a pointer that aliases `task`. As long as you don't read that
/// pointer except at syscall entry, you'll be okay.
pub unsafe fn set_current_task(task: &mut task::Task) {
    CURRENT_TASK_PTR = Some(NonNull::from(task));
}

static mut TICKS: u64 = 0;

/// Reads the tick counter.
pub fn now() -> Timestamp {
    Timestamp::from(unsafe { TICKS })
}

#[allow(unused_variables)]
pub fn disable_irq(n: u32) {}

#[allow(unused_variables)]
pub fn enable_irq(n: u32) {}

// Constants that may change depending on configuration
include!(concat!(env!("OUT_DIR"), "/consts.rs"));
