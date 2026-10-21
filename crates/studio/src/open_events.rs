//! Files opened from the Finder: a double-click on a `.scad`, "Open With",
//! or a file dropped on the Dock icon. macOS sends them as an "open
//! documents" Apple event rather than as arguments, and the windowing
//! library does not listen for it, so a handler is installed here.
//!
//! AppKit installs its own handler while the app finishes launching; ours
//! replaces it when the app announces it will finish launching, which is
//! before the event for the files that launched the app is handled.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use eframe::egui;

/// Paths received and not yet taken.
static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Wakes the window when files arrive while it is idle.
static WAKE: OnceLock<egui::Context> = OnceLock::new();

/// Takes the paths received since the last call.
pub fn take() -> Vec<PathBuf> {
    std::mem::take(&mut *OPENED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Repaints `ctx` when files arrive.
pub fn wake(ctx: &egui::Context) {
    let _ = WAKE.set(ctx.clone());
}

fn received(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).extend(paths);
    if let Some(ctx) = WAKE.get() {
        ctx.request_repaint();
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(target_os = "macos")]
pub use mac::install;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{AllocAnyThread, define_class, msg_send, sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSString};

    /// The "open documents" event ('aevt' 'odoc') and its file list ('----').
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "Code2FormOpenDocuments"]
        struct OpenDocuments;

        impl OpenDocuments {
            #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
            fn handle(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                let files: Option<Retained<NSAppleEventDescriptor>> =
                    unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
                let Some(files) = files else { return };
                let paths = (1..=files.numberOfItems())
                    .filter_map(|i| files.descriptorAtIndex(i)?.fileURLValue()?.path())
                    .map(|path| PathBuf::from(path.to_string()))
                    .collect();
                super::received(paths);
            }
        }
    );

    impl OpenDocuments {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Call before the event loop starts.
    pub fn install() {
        let on_launch = RcBlock::new(|_: NonNull<NSNotification>| {
            let handler = OpenDocuments::new();
            let manager = NSAppleEventManager::sharedAppleEventManager();
            unsafe {
                let () = msg_send![
                    &*manager,
                    setEventHandler: &*handler,
                    andSelector: sel!(handleOpenDocuments:withReplyEvent:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: OPEN_DOCUMENTS
                ];
            }
            // The event manager does not keep its handlers alive.
            std::mem::forget(handler);
        });
        let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
        let center = NSNotificationCenter::defaultCenter();
        let observer = unsafe { center.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &on_launch) };
        // Observing for the life of the app.
        std::mem::forget(observer);
    }
}
