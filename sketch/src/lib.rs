#![no_std]

// Our assembly language entry points
extern "C" {
    fn _sys_send(descriptor: &mut SendDescriptor<'_>) -> SendResponse;
}

/// A type for designating a task you want to interact with.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TaskName(pub u16);

/// Sends a message and waits for a reply.
/// 
/// The target task is named by `dest`. If `dest` is a name that is stale (i.e.
/// the target has reset since we last interacted), this returns
/// `DeathComesForUsAll`.
///
/// The request to transmit is identified by `request`. The contents of the
/// slice will be transferred by the kernel into a place defined by the
/// recipient if/when the message is delivered. If the recipient hasn't given
/// enough room for `request` in its entirety, you will not be informed of this,
/// but the recipient will.
///
/// `response` gives the buffer in which the response message, if any, should be
/// written. The message will be written if (1) this message is received and (2)
/// the recipient replies to us. If the message fits, its size is returned.
/// Otherwise, the first `response.len()` bytes are written and
/// `OverlyEnthusiasticResponse` is returned.
///
/// The `leases` table optionally makes sections of your address space visible
/// to the peer without additional copies. Leases are revoked before this
/// returns, so it's equivalent to borrowing.
pub fn send_untyped(
    dest: TaskName,
    request: &[u8],
    response: &mut [u8],
    leases: &[Lease<'_>],
) -> Result<usize, SendError> {
    let r = unsafe {
        _sys_send(&mut SendDescriptor {
            dest: dest.0,
            request_base: request.as_ptr(),
            request_len: request.len(),
            response_base: response.as_mut_ptr(),
            response_len: response.len(),
            lease_base: leases.as_ptr(),
            lease_len: leases.len(),
        })
    };
    if r.success {
        Ok(r.param)
    } else {
        Err(match r.param {
            0 => SendError::DeathComesForUsAll,
            1 => SendError::OverlyEnthusiasticResponse,
            _ => panic!(),
        })
    }
}

#[repr(C)]
struct SendDescriptor<'a> {
    dest: u16,
    request_base: *const u8,
    request_len: usize,
    response_base: *mut u8,
    response_len: usize,
    lease_base: *const Lease<'a>,
    lease_len: usize,
}

#[repr(C)]
struct SendResponse {
    success: bool,
    param: usize,
}

// TODO: this is a great start to a user-facing type but it needs to be FFI-safe
// if I'm going to ship it to the kernel
pub enum Lease<'a> {
    /// Indicates that you wish to give the peer temporary read access to this
    /// slice.
    Read(&'a [u8]),
    /// Indicates that you wish to give the peer temporary write access to this
    /// slice. (Just write! No reading. This means you don't have to defensively
    /// clear the buffer first to avoid leaks. If we need read/write we can add
    /// it later.)
    Write(&'a mut [u8]),
}

/// Things that can go wrong when sending, under *normal operation.*
///
/// Conditions that are conspicuously missing from this set:
///
/// - Can't send to that task because of MAC: I would rather treat any
///   MAC violation as a fault that gets escalated to supervision.
///
/// - Message larger than supported by kernel: message size limits are known at
///   compile time, and most messages are expected to be statically sized. An
///   attempt to send a message that's too big is a malfunction and should also
///   be treated as a fault.
///
/// - Attempt to send from, or receive into, sections of the address space that
///   you do not own: malfunction, fault.
///
/// Perhaps you are noticing a trend.
#[derive(Copy, Clone, Debug)]
pub enum SendError {
    /// The peer restarted since you last spoke to it. You might need to redo
    /// some work.
    DeathComesForUsAll,
    /// Your message was accepted and processed, but the peer returned a
    /// response that was larger than the buffer you offered. The prefix of the
    /// response has been deposited for your inspection.
    OverlyEnthusiasticResponse,
}