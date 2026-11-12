//! macOS hands a `bonk://` link to the running application as an Apple event, never as an argument.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{ClassType, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSObjectProtocol};

/// `kInternetEventClass` and `kAEGetURL` are both 'GURL'; `keyDirectObject` is '----'.
const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

/// Image identifiers received, read by the window at its next tick.
static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

define_class!(
    // SAFETY: NSObject has no subclassing requirement and the class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "BonkLinkHandler"]
    struct LinkHandler;

    unsafe impl NSObjectProtocol for LinkHandler {}

    impl LinkHandler {
        #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
        fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            // SAFETY: paramDescriptorForKeyword: takes a four-character code and returns a descriptor or nil.
            let param: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
            let Some(link) = param.and_then(|param| param.stringValue()) else {
                return;
            };
            match bonk_catalog::link::image_id(&link.to_string()) {
                Ok(id) => {
                    if let Ok(mut received) = RECEIVED.lock() {
                        received.push(id.to_owned());
                    }
                }
                Err(error) => log::warn!("bonk:// link refused: {error}"),
            }
        }
    }
);

/// Must run before the event loop starts, or the link that launched the application is lost.
pub(crate) fn listen() {
    // SAFETY: `new` returns a retained instance of a class defined above.
    let handler: Retained<LinkHandler> = unsafe { msg_send![LinkHandler::class(), new] };
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // SAFETY: the handler answers the selector with the signature Apple events expect.
    unsafe {
        let () = msg_send![
            &*manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleGetURLEvent:withReplyEvent:),
            forEventClass: GET_URL,
            andEventID: GET_URL
        ];
    }
    // The manager does not retain its handler: it lives as long as the process.
    std::mem::forget(handler);
}

pub(crate) fn take() -> Vec<String> {
    RECEIVED
        .lock()
        .map(|mut received| std::mem::take(&mut *received))
        .unwrap_or_default()
}
