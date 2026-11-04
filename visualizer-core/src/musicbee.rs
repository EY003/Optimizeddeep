use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

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
/// any SMTC-aware player (MusicBee with the mb_MediaControl plugin, Spotify,
/// browsers, VLC, Groove, etc.).
pub fn spawn_smtc_listener(
    tx: Sender<TrackInfo>,
    running: Arc<AtomicBool>,
    session_count: Arc<AtomicUsize>,
) {
    std::thread::spawn(move || {
        // ── 1. Initialize COM on this worker thread ───────────
        // SMTC (WinRT) is built on COM and requires an initialized apartment.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        // ── 2. Acquire the session manager, retrying on failure ─
        // On cold boot the SMTC service may not be ready immediately.
        let manager = loop {
            if !running.load(Ordering::Relaxed) {
                return;
            }
            match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                .and_then(|op| op.get())
            {
                Ok(m) => break m,
                Err(e) => {
                    eprintln!("SMTC: failed to get session manager: {:?} (retrying)", e);
                    std::thread::sleep(Duration::from_millis(1000));
                }
            }
        };

        eprintln!("SMTC: session manager acquired");

        let mut last_title = String::new();
        let mut last_artist = String::new();
        let mut last_album = String::new();

        // ── 3. Poll loop ──────────────────────────────────────
        while running.load(Ordering::Relaxed) {
            match manager.GetSessions() {
                Ok(sessions) => {
                    let count = sessions.Size().unwrap_or(0) as usize;
                    session_count.store(count, Ordering::Relaxed);

                    let session = pick_best_session(&sessions);

                    if let Some(session) = session {
                        if let Ok(props) =
                            session.TryGetMediaPropertiesAsync().and_then(|op| op.get())
                        {
                            let title =
                                props.Title().map(|s| s.to_string()).unwrap_or_default();
                            let artist =
                                props.Artist().map(|s| s.to_string()).unwrap_or_default();
                            let album = props
                                .AlbumTitle()
                                .map(|s| s.to_string())
                                .unwrap_or_default();

                            if title != last_title
                                || artist != last_artist
                                || album != last_album
                            {
                                let cover_art = read_thumbnail(&props);
                                last_title = title.clone();
                                last_artist = artist.clone();
                                last_album = album.clone();

                                eprintln!(
                                    "SMTC: now playing → {} — {} (cover: {})",
                                    artist,
                                    title,
                                    cover_art
                                        .as_ref()
                                        .map(|b| format!("{} bytes", b.len()))
                                        .unwrap_or_else(|| "none".into())
                                );

                                let _ = tx.send(TrackInfo {
                                    title,
                                    artist,
                                    album,
                                    cover_art,
                                });
                            }
                        }
                    } else if !last_title.is_empty() {
                        // Nothing is playing anymore — clear the UI.
                        last_title.clear();
                        last_artist.clear();
                        last_album.clear();
                        let _ = tx.send(TrackInfo {
                            title: String::new(),
                            artist: String::new(),
                            album: String::new(),
                            cover_art: None,
                        });
                    }
                }
                Err(e) => {
                    eprintln!("SMTC: GetSessions failed: {:?}", e);
                    session_count.store(0, Ordering::Relaxed);
                }
            }

            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Prefer a session whose playback status is `Playing`; otherwise fall back
/// to the first session that has any usable properties.
fn pick_best_session(
    sessions: &windows::Foundation::Collections::IVectorView<
        GlobalSystemMediaTransportControlsSession,
    >,
) -> Option<GlobalSystemMediaTransportControlsSession> {
    let count = sessions.Size().ok()?;
    let mut fallback: Option<GlobalSystemMediaTransportControlsSession> = None;

    for idx in 0..count {
        let session = match sessions.GetAt(idx) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if is_playing(&session) {
            return Some(session);
        }

        if fallback.is_none() {
            fallback = Some(session);
        }
    }

    fallback
}

fn is_playing(session: &GlobalSystemMediaTransportControlsSession) -> bool {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .map(|status| status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false)
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
