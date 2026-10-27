use std::{
    ffi::CString,
    io,
    net::{IpAddr, TcpListener},
    os::fd::FromRawFd,
    ptr,
};

use crate::{error, Result};

#[link(name = "System")]
unsafe extern "C" {
    fn launch_activate_socket(
        name: *const libc::c_char,
        fds: *mut *mut libc::c_int,
        count: *mut libc::size_t,
    ) -> libc::c_int;
}

pub fn listener(port: u16) -> Result<TcpListener> {
    let name = CString::new("Listener")?;
    let mut fds = ptr::null_mut();
    let mut count = 0;
    // launchd allocates the descriptor array; each descriptor belongs to us.
    let code = unsafe { launch_activate_socket(name.as_ptr(), &mut fds, &mut count) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code).into());
    }
    if fds.is_null() {
        return Err(error("launchd returned no socket descriptors"));
    }
    let descriptors = unsafe { std::slice::from_raw_parts(fds, count) };
    let owned: Vec<TcpListener> = descriptors
        .iter()
        .map(|&fd| unsafe { TcpListener::from_raw_fd(fd) })
        .collect();
    unsafe { libc::free(fds.cast()) };
    if owned.len() != 1 {
        return Err(error("expected exactly one launchd Listener socket"));
    }
    let listener = owned.into_iter().next().expect("checked length");
    let address = listener.local_addr()?;
    if address.port() != port || address.ip() != IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) {
        return Err(error(format!("unexpected launchd listener: {address}")));
    }
    Ok(listener)
}
