//! What only Android can do for the store, asked for through its Java side.
//!
//! The store is a native program inside one Android activity. Handing something to
//! another app (a page to Google Play or the browser) is done the way every Android
//! app does it: an intent to view an address, started from the activity.
//!
//! [Run for real in an Android 14 emulator. Not tried on a phone or an Android TV.]

use bilby_core::{Error, Result};
use jni::objects::{JObject, JValue};
use slint::android::AndroidApp;
use std::sync::OnceLock;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Keep hold of the activity, once, at start-up.
pub fn remember(app: &AndroidApp) {
    let _ = APP.set(app.clone());
}

/// Ask Android to show an address: a web page in the browser, an app's page in
/// Google Play. Only https addresses; the caller has checked which.
pub fn view(url: &str) -> Result<()> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Unsupported(format!("Refusing to open \"{url}\": not an https link")));
    }
    let app = APP.get().ok_or_else(|| Error::Unsupported("Android is not ready to open things yet.".to_string()))?;
    let failed = |what: &str| Error::Unsupported(format!("Android could not open it: {what}"));
    // The virtual machine and the activity are Android's own; they outlive this call.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| failed(&e.to_string()))?;
    let mut env = vm.attach_current_thread().map_err(|e| failed(&e.to_string()))?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let outcome = (|| -> jni::errors::Result<()> {
        let address = env.new_string(url)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&address)],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        // A new task: the page belongs to the app that shows it, not to the store.
        const NEW_TASK: i32 = 0x1000_0000;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(NEW_TASK)])?;
        env.call_method(&activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    })();
    // Nothing on the device can show it (no browser, no store): Java says so by
    // throwing, which must be cleared before anything else is asked of it.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
        return Err(failed("nothing on this device can show that address"));
    }
    outcome.map_err(|e| failed(&e.to_string()))
}
