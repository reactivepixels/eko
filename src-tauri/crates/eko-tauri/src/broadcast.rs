//! macOS distributed-notification broadcast of EKO's playback state, for companion apps
//! (native notification, not a Tauri event — reaches processes outside the webview).
//!
//! Mirrors the shape of Spotify's `com.spotify.client.PlaybackStateChanged` so a consumer
//! that already parses Spotify's notification can handle EKO's with the same code.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSDistributedNotificationCenter, NSString};

const NOTIFICATION_NAME: &str = "com.reactivepixels.eko.playbackState";

/// Post `{ "Player State": state, "Name": name, "Artist": artist }` on the distributed
/// notification center. `state` is expected to be `"Playing"` / `"Paused"` / `"Stopped"`.
pub fn post(state: &str, name: &str, artist: &str) {
    let notification_name = NSString::from_str(NOTIFICATION_NAME);

    let key_state = NSString::from_str("Player State");
    let key_name = NSString::from_str("Name");
    let key_artist = NSString::from_str("Artist");
    let val_state = NSString::from_str(state);
    let val_name = NSString::from_str(name);
    let val_artist = NSString::from_str(artist);

    let user_info: Retained<NSDictionary<NSString, NSString>> = NSDictionary::from_slices(
        &[&*key_state, &*key_name, &*key_artist],
        &[&*val_state, &*val_name, &*val_artist],
    );
    // SAFETY: reinterprets the dictionary's generic key/value types as the erased
    // `AnyObject` the raw ObjC method expects; the underlying object (an NSDictionary of
    // NSStrings) is unchanged, so this is a type-level erasure only.
    let user_info: &NSDictionary = unsafe { user_info.cast_unchecked::<AnyObject, AnyObject>() };

    let center = NSDistributedNotificationCenter::defaultCenter();
    // SAFETY: `user_info` is a valid `NSDictionary<NSString, NSString>` reinterpreted to
    // the method's erased signature, matching what callers of this notification expect.
    unsafe {
        center.postNotificationName_object_userInfo_deliverImmediately(
            &notification_name,
            None,
            Some(user_info),
            true,
        );
    }
}
