//! Names for machines on this Mac: `ubuntu.patina.local` -> its address.
//!
//! The engine registers an A record for each running machine with
//! mDNSResponder, local to this Mac (`kDNSServiceInterfaceIndexLocalOnly`),
//! so the names resolve in browsers and terminals here but are never
//! announced on the network. They last while the engine's connection to
//! mDNSResponder does, so they go away when the engine stops.

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::net::Ipv4Addr;
use std::os::raw::{c_char, c_int};

type DnsRef = *mut c_void;
type RecordRef = *mut c_void;
type Reply = extern "C" fn(DnsRef, RecordRef, u32, i32, *mut c_void);

#[link(name = "System")]
extern "C" {
    fn DNSServiceCreateConnection(sd: *mut DnsRef) -> i32;
    fn DNSServiceRegisterRecord(
        sd: DnsRef,
        record: *mut RecordRef,
        flags: u32,
        interface: u32,
        fullname: *const c_char,
        rrtype: u16,
        rrclass: u16,
        rdlen: u16,
        rdata: *const c_void,
        ttl: u32,
        reply: Reply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRemoveRecord(sd: DnsRef, record: RecordRef, flags: u32) -> i32;
    fn DNSServiceRefSockFD(sd: DnsRef) -> c_int;
    fn DNSServiceProcessResult(sd: DnsRef) -> i32;
    fn DNSServiceRefDeallocate(sd: DnsRef);
}

const UNIQUE: u32 = 0x20;
const LOCAL_ONLY: u32 = u32::MAX;
const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;

extern "C" fn ignore(_: DnsRef, _: RecordRef, _: u32, _: i32, _: *mut c_void) {}

/// The registered names. Only used from one task at a time.
pub struct Names {
    sd: DnsRef,
    records: HashMap<String, (Ipv4Addr, RecordRef)>,
}

// The handles are only touched through `&mut self`.
unsafe impl Send for Names {}

impl Names {
    pub fn new() -> Option<Names> {
        let mut sd: DnsRef = std::ptr::null_mut();
        let e = unsafe { DNSServiceCreateConnection(&mut sd) };
        (e == 0 && !sd.is_null()).then(|| Names { sd, records: HashMap::new() })
    }

    /// Make the registered names exactly `want` (name -> address). Returns
    /// false when mDNSResponder went away; make a new `Names` then.
    pub fn set(&mut self, want: &HashMap<String, Ipv4Addr>) -> bool {
        if !self.drain() {
            return false;
        }
        let stale: Vec<String> = self.records.iter().filter(|(n, (ip, _))| want.get(*n) != Some(ip)).map(|(n, _)| n.clone()).collect();
        for n in stale {
            if let Some((_, rec)) = self.records.remove(&n) {
                unsafe { DNSServiceRemoveRecord(self.sd, rec, 0) };
            }
        }
        for (name, ip) in want {
            if self.records.contains_key(name) {
                continue;
            }
            let Ok(cname) = CString::new(name.as_str()) else { continue };
            let data = ip.octets();
            let mut rec: RecordRef = std::ptr::null_mut();
            let e = unsafe {
                DNSServiceRegisterRecord(self.sd, &mut rec, UNIQUE, LOCAL_ONLY, cname.as_ptr(), TYPE_A, CLASS_IN, 4, data.as_ptr().cast(), 60, ignore, std::ptr::null_mut())
            };
            if e == 0 {
                self.records.insert(name.clone(), (*ip, rec));
            }
        }
        true
    }

    /// Read mDNSResponder's replies so they don't pile up, without waiting.
    fn drain(&mut self) -> bool {
        let fd = unsafe { DNSServiceRefSockFD(self.sd) };
        if fd < 0 {
            return false;
        }
        loop {
            let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut p, 1, 0) } <= 0 || p.revents & libc::POLLIN == 0 {
                return p.revents & (libc::POLLHUP | libc::POLLERR) == 0;
            }
            if unsafe { DNSServiceProcessResult(self.sd) } != 0 {
                return false;
            }
        }
    }
}

impl Drop for Names {
    fn drop(&mut self) {
        // Deallocating the connection removes its records.
        unsafe { DNSServiceRefDeallocate(self.sd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to this Mac's mDNSResponder: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn names_resolve_on_this_mac() {
        use std::net::ToSocketAddrs;
        let mut n = Names::new().expect("mDNSResponder");
        let name = format!("patina-test-{}.patina.local", std::process::id());
        let ip = Ipv4Addr::new(192, 168, 64, 250);
        assert!(n.set(&HashMap::from([(name.clone(), ip)])));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let got: Vec<_> = (name.as_str(), 80).to_socket_addrs().unwrap().map(|a| a.ip()).collect();
        assert!(got.contains(&std::net::IpAddr::V4(ip)), "{got:?}");
        assert!(n.set(&HashMap::new()));
        assert!(n.records.is_empty());
    }
}
