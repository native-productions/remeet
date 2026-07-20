//! Detects when *another* app is holding a call — mic and speakers both live.
//!
//! This is the "you forgot to hit record" signal. It watches the default input and
//! output devices for CoreAudio's `kAudioDevicePropertyDeviceIsRunningSomewhere`,
//! which is true whenever *any* process on the machine has the device running. When
//! both go live at once — someone talking into the mic while sound plays out — a
//! call is almost certainly happening, and the owner is told.
//!
//! Event-driven, not polled: the HAL calls back only on a state change, so an idle
//! machine costs nothing. The listeners are serviced on the HAL's own thread (the
//! run loop is detached below), so the callback here must be thread-safe and must
//! not assume it runs on the main thread.
//!
//! This is CoreAudio C FFI. It stops at [`CallWatcher`]; everything above sees a
//! plain `Fn(bool)` that fires with `true` when a call starts and `false` when it
//! ends.

use std::os::raw::c_void;
use std::ptr;
use std::sync::Mutex;

/// A four-character code as CoreAudio spells its selectors, big-endian.
const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

const SYSTEM_OBJECT: u32 = 1; // kAudioObjectSystemObject
const DEFAULT_INPUT: u32 = fourcc(b"dIn "); // kAudioHardwarePropertyDefaultInputDevice
const DEFAULT_OUTPUT: u32 = fourcc(b"dOut"); // kAudioHardwarePropertyDefaultOutputDevice
const RUNNING_SOMEWHERE: u32 = fourcc(b"gone"); // kAudioDevicePropertyDeviceIsRunningSomewhere
const SCOPE_GLOBAL: u32 = fourcc(b"glob"); // kAudioObjectPropertyScopeGlobal
const ELEMENT_MAIN: u32 = 0; // kAudioObjectPropertyElementMain
const RUN_LOOP: u32 = fourcc(b"rnlp"); // kAudioHardwarePropertyRunLoop
const UNKNOWN_DEVICE: u32 = 0; // kAudioObjectUnknown

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

type ListenerProc =
    extern "C" fn(u32, u32, *const PropertyAddress, *mut c_void) -> i32;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        id: u32,
        addr: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        io_size: *mut u32,
        out: *mut c_void,
    ) -> i32;

    fn AudioObjectSetPropertyData(
        id: u32,
        addr: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;

    fn AudioObjectAddPropertyListener(
        id: u32,
        addr: *const PropertyAddress,
        proc: ListenerProc,
        client: *mut c_void,
    ) -> i32;

    fn AudioObjectRemovePropertyListener(
        id: u32,
        addr: *const PropertyAddress,
        proc: ListenerProc,
        client: *mut c_void,
    ) -> i32;
}

fn address(selector: u32) -> PropertyAddress {
    PropertyAddress {
        selector,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    }
}

/// Reads the current default input or output device id, or `0` if there is none.
fn default_device(selector: u32) -> u32 {
    let addr = address(selector);
    let mut id: u32 = UNKNOWN_DEVICE;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            &mut size,
            (&mut id as *mut u32).cast(),
        )
    };
    if status == 0 { id } else { UNKNOWN_DEVICE }
}

/// Whether any process is currently running this device.
fn is_running(device: u32) -> bool {
    if device == UNKNOWN_DEVICE {
        return false;
    }
    let addr = address(RUNNING_SOMEWHERE);
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &addr,
            0,
            ptr::null(),
            &mut size,
            (&mut value as *mut u32).cast(),
        )
    };
    status == 0 && value != 0
}

/// Tells the HAL to service property listeners on its own thread instead of the
/// process's main run loop. Without this the callbacks would only fire while the
/// main run loop happens to be spinning; with it they fire regardless of what the
/// app's UI thread is doing.
fn detach_run_loop() {
    let addr = address(RUN_LOOP);
    let null_run_loop: *const c_void = ptr::null();
    unsafe {
        AudioObjectSetPropertyData(
            SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            size_of::<*const c_void>() as u32,
            (&null_run_loop as *const *const c_void).cast(),
        );
    }
}

unsafe fn add_listener(object: u32, selector: u32, client: *mut c_void) {
    if object == UNKNOWN_DEVICE {
        return;
    }
    let addr = address(selector);
    unsafe { AudioObjectAddPropertyListener(object, &addr, listener, client) };
}

unsafe fn remove_listener(object: u32, selector: u32, client: *mut c_void) {
    if object == UNKNOWN_DEVICE {
        return;
    }
    let addr = address(selector);
    unsafe { AudioObjectRemovePropertyListener(object, &addr, listener, client) };
}

/// The watcher's state, reached by the C callback through a raw pointer and guarded
/// by a mutex because the callback thread and `Drop` can both touch it.
struct Inner {
    on_change: Box<dyn Fn(bool) + Send + Sync>,
    /// The devices the running-state listeners are currently attached to. Tracked so
    /// they can be moved when the default device changes (headphones plugged in).
    input: u32,
    output: u32,
    /// The last state handed to `on_change`, so only real transitions are reported.
    last_active: bool,
}

/// Recomputes everything on any change: re-reads the default devices, moves the
/// running listeners if they switched, then reports a call starting or ending.
fn reconcile(inner: &mut Inner, client: *mut c_void) {
    let new_input = default_device(DEFAULT_INPUT);
    if new_input != inner.input {
        unsafe {
            remove_listener(inner.input, RUNNING_SOMEWHERE, client);
            add_listener(new_input, RUNNING_SOMEWHERE, client);
        }
        inner.input = new_input;
    }

    let new_output = default_device(DEFAULT_OUTPUT);
    if new_output != inner.output {
        unsafe {
            remove_listener(inner.output, RUNNING_SOMEWHERE, client);
            add_listener(new_output, RUNNING_SOMEWHERE, client);
        }
        inner.output = new_output;
    }

    let active = is_running(inner.input) && is_running(inner.output);
    if active != inner.last_active {
        inner.last_active = active;
        (inner.on_change)(active);
    }
}

extern "C" fn listener(
    _object: u32,
    _count: u32,
    _addresses: *const PropertyAddress,
    client: *mut c_void,
) -> i32 {
    let ptr = client as *const Mutex<Inner>;
    if ptr.is_null() {
        return 0;
    }
    // Safe: the pointer stays valid until `CallWatcher::drop` removes every listener
    // under the lock, so no callback can be in flight against freed memory.
    let mutex = unsafe { &*ptr };
    if let Ok(mut inner) = mutex.lock() {
        reconcile(&mut inner, client);
    }
    0
}

/// Watches for another app holding a call and reports when one starts or ends.
///
/// Lives for as long as it is held; dropping it removes the CoreAudio listeners.
pub struct CallWatcher {
    inner: *mut Mutex<Inner>,
}

// The pointee is a `Mutex<Inner>` whose `Inner` is `Send`, so the only access from
// other threads — the callback — is synchronised. `Drop` is the sole `&mut` path and
// runs after every listener is removed.
unsafe impl Send for CallWatcher {}
unsafe impl Sync for CallWatcher {}

impl CallWatcher {
    /// Starts watching. `on_change` fires with `true` when the mic and speakers both
    /// go live (a call is likely) and `false` when either drops. It is called from
    /// the HAL's notification thread, so it must be cheap and thread-safe.
    ///
    /// If a call is already in progress at startup, `on_change(true)` fires once
    /// immediately, on the calling thread.
    pub fn start<F>(on_change: F) -> Self
    where
        F: Fn(bool) + Send + Sync + 'static,
    {
        detach_run_loop();

        let input = default_device(DEFAULT_INPUT);
        let output = default_device(DEFAULT_OUTPUT);
        let active = is_running(input) && is_running(output);

        let inner = Box::into_raw(Box::new(Mutex::new(Inner {
            on_change: Box::new(on_change),
            input,
            output,
            // Start from `false` so an already-active call reports once below.
            last_active: false,
        })));
        let client = inner as *mut c_void;

        unsafe {
            add_listener(input, RUNNING_SOMEWHERE, client);
            add_listener(output, RUNNING_SOMEWHERE, client);
            // The default device can change under us; these fire the reconcile that
            // moves the running listeners onto the new device.
            add_listener(SYSTEM_OBJECT, DEFAULT_INPUT, client);
            add_listener(SYSTEM_OBJECT, DEFAULT_OUTPUT, client);
        }

        if active && let Ok(mut guard) = unsafe { &*inner }.lock() {
            guard.last_active = true;
            (guard.on_change)(true);
        }

        Self { inner }
    }
}

impl Drop for CallWatcher {
    fn drop(&mut self) {
        let client = self.inner as *mut c_void;
        // Remove the listeners before freeing so no callback can start afterwards. A
        // callback already running holds the lock; taking it here waits it out.
        if let Ok(guard) = unsafe { &*self.inner }.lock() {
            unsafe {
                remove_listener(guard.input, RUNNING_SOMEWHERE, client);
                remove_listener(guard.output, RUNNING_SOMEWHERE, client);
                remove_listener(SYSTEM_OBJECT, DEFAULT_INPUT, client);
                remove_listener(SYSTEM_OBJECT, DEFAULT_OUTPUT, client);
            }
        }
        drop(unsafe { Box::from_raw(self.inner) });
    }
}
