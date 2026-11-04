use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
};
use windows::Storage::Streams::DataReader;

#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_art: Option<Vec<u8>>,
}

/// Spawns a background thread that polls Windows' System Media Transport
/// Controls (SMTC) for the current track and cover art.
///
/// SMTC is the same API used by the Windows volume overlay, so this works with
/// MusicBee, Spotify, browsers, VLC, Groove, etc. — no plugin or TCP server
/// required.
pub fn spawn_smtc_listener(tx: Sender<TrackInfo>, running: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .and_then(|op| op.get())
        {
            Ok(m) => m,
            Err(e) => {
                eprintln!("SMTC: failed to get session manager: {:?}", e);
                return;
            }
        };

        let mut last_title = String::new();
        let mut last_artist = String::new();
        let mut last_album = String::new();

        while running.load(Ordering::Relaxed) {
            if let Ok(session) = manager.GetCurrentSession() {
                if let Ok(props) =
                    session.TryGetMediaPropertiesAsync().and_then(|op| op.get())
                {
                    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
                    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
                    let album =
                        props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();

                    // Only send when the track actually changes (avoids spamming
                    // the UI thread during normal playback).
                    if title != last_title || artist != last_artist || album != last_album {
                        let cover_art = read_thumbnail(&props);
                        last_title = title.clone();
                        last_artist = artist.clone();
                        last_album = album.clone();

                        let _ = tx.send(TrackInfo {
                            title,
                            artist,
                            album,
                            cover_art,
                        });
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

fn read_thumbnail(
    props: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<Vec<u8>> {
    let thumb_ref = props.Thumbnail().ok()?;
    let stream = thumb_ref.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size as usize];
    unsafe {
        reader.ReadBytes(&mut bytes).ok()?;
    }
    Some(bytes)
}
