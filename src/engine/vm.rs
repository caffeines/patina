//! The Linux VM, built with Apple's Virtualization framework.
//!
//! Every Virtualization call happens on the main dispatch queue, which the
//! engine process runs with `dispatch_main()`. Other threads reach the VM
//! through [`on_main`] and [`connect`].

use std::cell::Cell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::fd::{IntoRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_foundation::{NSArray, NSError, NSFileHandle, NSString, NSURL};
use objc2_virtualization::*;

pub struct Spec {
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk: PathBuf,
    pub seed: PathBuf,
    pub efi: PathBuf,
    pub console: PathBuf,
    pub home: PathBuf,
    pub mac: String,
    pub rosetta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    None,
    Stopped,
    Running,
    Paused,
    Error,
    Starting,
    Stopping,
    Other,
}

/// Objects that may only be touched on the main dispatch queue.
///
/// `dispatch_main()` ends the original main thread and drains the main queue
/// on pool threads, so thread-locals won't do: the VM would be freed with the
/// thread. The main queue is serial, so a global behind a mutex is safe as
/// long as it's only used from there.
struct MainOnly<T>(T);
unsafe impl<T> Send for MainOnly<T> {}

static VM: Mutex<Option<MainOnly<Retained<VZVirtualMachine>>>> = Mutex::new(None);
static CONNS: Mutex<Option<HashMap<u64, MainOnly<Retained<VZVirtioSocketConnection>>>>> = Mutex::new(None);
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

fn the_vm() -> Option<Retained<VZVirtualMachine>> {
    VM.lock().unwrap().as_ref().map(|v| v.0.clone())
}

fn url(p: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
}

fn describe(e: &NSError) -> String {
    e.localizedDescription().to_string()
}

/// Run `f` on the main queue and wait for its result. Never call this from the main thread.
pub fn on_main<R: Send + 'static>(f: impl FnOnce() -> R + Send) -> R {
    let slot: Mutex<Option<R>> = Mutex::new(None);
    DispatchQueue::main().exec_sync(|| {
        *slot.lock().unwrap() = Some(f());
    });
    slot.into_inner().unwrap().expect("main queue result")
}

fn build(spec: &Spec) -> Result<Retained<VZVirtualMachineConfiguration>, String> {
    unsafe {
        let cfg = VZVirtualMachineConfiguration::new();
        let max_cpu = VZVirtualMachineConfiguration::maximumAllowedCPUCount() as u32;
        cfg.setCPUCount(spec.cpus.clamp(1, max_cpu.max(1)) as usize);
        let min_mem = VZVirtualMachineConfiguration::minimumAllowedMemorySize();
        cfg.setMemorySize(((spec.memory_gb as u64) << 30).max(min_mem));
        cfg.setPlatform(&VZGenericPlatformConfiguration::new());

        // UEFI boot, with a persistent variable store.
        let store = if spec.efi.exists() {
            VZEFIVariableStore::initWithURL(VZEFIVariableStore::alloc(), &url(&spec.efi))
        } else {
            VZEFIVariableStore::initCreatingVariableStoreAtURL_options_error(
                VZEFIVariableStore::alloc(),
                &url(&spec.efi),
                VZEFIVariableStoreInitializationOptions::empty(),
            )
            .map_err(|e| format!("EFI variable store: {}", describe(&e)))?
        };
        let boot = VZEFIBootLoader::new();
        boot.setVariableStore(Some(&store));
        cfg.setBootLoader(Some(&boot));

        // Disks: the system disk, and the first-boot seed.
        let mut storage: Vec<Retained<VZStorageDeviceConfiguration>> = Vec::new();
        // Explicit caching and fsync: the default ("automatic") mode can hand
        // ext4 stale data under heavy writes, which corrupts the filesystem.
        let disk = VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &url(&spec.disk),
            false,
            VZDiskImageCachingMode::Cached,
            VZDiskImageSynchronizationMode::Fsync,
        )
        .map_err(|e| format!("Disk: {}", describe(&e)))?;
        storage.push(Retained::into_super(VZVirtioBlockDeviceConfiguration::initWithAttachment(VZVirtioBlockDeviceConfiguration::alloc(), &disk)));
        if spec.seed.exists() {
            let seed = VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(VZDiskImageStorageDeviceAttachment::alloc(), &url(&spec.seed), true)
                .map_err(|e| format!("Seed: {}", describe(&e)))?;
            storage.push(Retained::into_super(VZVirtioBlockDeviceConfiguration::initWithAttachment(VZVirtioBlockDeviceConfiguration::alloc(), &seed)));
        }
        cfg.setStorageDevices(&NSArray::from_retained_slice(&storage));

        // NAT networking with a stable MAC address.
        let net = VZVirtioNetworkDeviceConfiguration::new();
        net.setAttachment(Some(&VZNATNetworkDeviceAttachment::new()));
        if let Some(mac) = VZMACAddress::initWithString(VZMACAddress::alloc(), &NSString::from_str(&spec.mac)) {
            net.setMACAddress(&mac);
        }
        cfg.setNetworkDevices(&NSArray::from_retained_slice(&[Retained::into_super(net)]));

        // Console output goes to a log file.
        let file = OpenOptions::new().create(true).append(true).open(&spec.console).map_err(|e| format!("Console log: {e}"))?;
        let fd = file.into_raw_fd();
        let write = NSFileHandle::initWithFileDescriptor_closeOnDealloc(NSFileHandle::alloc(), fd, true);
        let attachment = VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
            VZFileHandleSerialPortAttachment::alloc(),
            None,
            Some(&write),
        );
        let console = VZVirtioConsoleDeviceSerialPortConfiguration::new();
        console.setAttachment(Some(&attachment));
        cfg.setSerialPorts(&NSArray::from_retained_slice(&[Retained::into_super(console)]));

        // Shared folders: the home folder, and Rosetta for amd64 binaries.
        let mut shares: Vec<Retained<VZDirectorySharingDeviceConfiguration>> = Vec::new();
        let home = VZVirtioFileSystemDeviceConfiguration::initWithTag(VZVirtioFileSystemDeviceConfiguration::alloc(), &NSString::from_str("patina-home"));
        let dir = VZSharedDirectory::initWithURL_readOnly(VZSharedDirectory::alloc(), &url(&spec.home), false);
        home.setShare(Some(&VZSingleDirectoryShare::initWithDirectory(VZSingleDirectoryShare::alloc(), &dir)));
        shares.push(Retained::into_super(home));
        if spec.rosetta && VZLinuxRosettaDirectoryShare::availability() == VZLinuxRosettaAvailability::Installed {
            if let Ok(rosetta) = VZLinuxRosettaDirectoryShare::initWithError(VZLinuxRosettaDirectoryShare::alloc()) {
                let fs = VZVirtioFileSystemDeviceConfiguration::initWithTag(VZVirtioFileSystemDeviceConfiguration::alloc(), &NSString::from_str("rosetta"));
                fs.setShare(Some(&rosetta));
                shares.push(Retained::into_super(fs));
            }
        }
        cfg.setDirectorySharingDevices(&NSArray::from_retained_slice(&shares));

        cfg.setSocketDevices(&NSArray::from_retained_slice(&[Retained::into_super(VZVirtioSocketDeviceConfiguration::new())]));
        cfg.setEntropyDevices(&NSArray::from_retained_slice(&[Retained::into_super(VZVirtioEntropyDeviceConfiguration::new())]));
        cfg.setMemoryBalloonDevices(&NSArray::from_retained_slice(&[Retained::into_super(
            VZVirtioTraditionalMemoryBalloonDeviceConfiguration::new(),
        )]));

        cfg.validateWithError().map_err(|e| describe(&e))?;
        Ok(cfg)
    }
}

/// Build and start the VM. Must run on the main thread.
pub fn start(spec: &Spec, done: impl Fn(Result<(), String>) + 'static) -> Result<(), String> {
    let cfg = build(spec)?;
    let vm = unsafe { VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &cfg) };
    let block = RcBlock::new(move |err: *mut NSError| {
        if err.is_null() {
            done(Ok(()));
        } else {
            done(Err(unsafe { describe(&*err) }));
        }
    });
    unsafe { vm.startWithCompletionHandler(&block) };
    *VM.lock().unwrap() = Some(MainOnly(vm));
    Ok(())
}

/// The VM's state. Must run on the main thread.
pub fn state() -> State {
    {
        let Some(vm) = the_vm() else { return State::None };
        let s = unsafe { vm.state() };
        if s == VZVirtualMachineState::Stopped {
            State::Stopped
        } else if s == VZVirtualMachineState::Running {
            State::Running
        } else if s == VZVirtualMachineState::Paused {
            State::Paused
        } else if s == VZVirtualMachineState::Error {
            State::Error
        } else if s == VZVirtualMachineState::Starting {
            State::Starting
        } else if s == VZVirtualMachineState::Stopping {
            State::Stopping
        } else {
            State::Other
        }
    }
}

/// Ask the guest to shut down (like pressing the power button). Main thread.
pub fn request_stop() -> Result<(), String> {
    let Some(vm) = the_vm() else { return Ok(()) };
    unsafe {
        if vm.canRequestStop() {
            vm.requestStopWithError().map_err(|e| describe(&e))
        } else {
            Err("The VM can't be asked to stop right now.".into())
        }
    }
}

/// Turn the VM off immediately. Main thread.
pub fn force_stop() {
    let Some(vm) = the_vm() else { return };
    unsafe {
        if vm.canStop() {
            let block = RcBlock::new(|_err: *mut NSError| {});
            vm.stopWithCompletionHandler(&block);
        }
    }
}

/// Open a vsock connection to `port` in the guest. Returns a duplicated file
/// descriptor and an id to pass to [`release`] when done.
pub async fn connect(port: u32) -> Result<(RawFd, u64), String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(RawFd, u64), String>>();
    DispatchQueue::main().exec_async(move || {
        let tx = Cell::new(Some(tx));
        let fail = |tx: &Cell<Option<tokio::sync::oneshot::Sender<_>>>, msg: String| {
            if let Some(tx) = tx.take() {
                let _ = tx.send(Err(msg));
            }
        };
        let vm = the_vm();
        let Some(vm) = vm else {
            fail(&tx, "The VM isn't running.".into());
            return;
        };
        let devices = unsafe { vm.socketDevices() };
        let Some(device) = devices.firstObject() else {
            fail(&tx, "The VM has no socket device.".into());
            return;
        };
        let Ok(device) = device.downcast::<VZVirtioSocketDevice>() else {
            fail(&tx, "Unexpected socket device.".into());
            return;
        };
        let block = RcBlock::new(move |conn: *mut VZVirtioSocketConnection, err: *mut NSError| {
            let Some(tx) = tx.take() else { return };
            if conn.is_null() {
                let msg = if err.is_null() { "Connection failed.".to_string() } else { unsafe { describe(&*err) } };
                let _ = tx.send(Err(msg));
                return;
            }
            let Some(conn) = (unsafe { Retained::retain(conn) }) else {
                let _ = tx.send(Err("Connection failed.".into()));
                return;
            };
            let fd = unsafe { libc::dup(conn.fileDescriptor()) };
            if fd < 0 {
                let _ = tx.send(Err("Couldn't use the connection.".into()));
                return;
            }
            let id = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
            CONNS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, MainOnly(conn));
            let _ = tx.send(Ok((fd, id)));
        });
        unsafe { device.connectToPort_completionHandler(port, &block) };
    });
    rx.await.unwrap_or_else(|_| Err("The engine is shutting down.".into()))
}

/// Drop a vsock connection opened with [`connect`].
pub fn release(id: u64) {
    DispatchQueue::main().exec_async(move || {
        if let Some(map) = CONNS.lock().unwrap().as_mut() {
            map.remove(&id);
        }
    });
}
