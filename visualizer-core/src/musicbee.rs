use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use serde_json::Value;
use base64::Engine;
use crossbeam_channel::Sender;

pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover_art: Option<Vec<u8>>,
}

pub async fn connect_mbrc(tx: Sender<TrackInfo>) {
    loop {
        if let Ok(stream) = TcpStream::connect("127.0.0.1:3000").await {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            // Handshake
            let _ = writer.write_all(b"player\r\n").await;
            let _ = writer.write_all(b"protocol 2\r\n").await;
            let _ = writer.write_all(b"init\r\n").await;

            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 { break; }
                if let Ok(json) = serde_json::from_str::<Value>(&line) {
                    let mut track = TrackInfo {
                        title: json["title"].as_str().unwrap_or("").to_string(),
                        artist: json["artist"].as_str().unwrap_or("").to_string(),
                        album: json["album"].as_str().unwrap_or("").to_string(),
                        cover_art: None,
                    };
                    
                    if let Some(b64) = json["cover"].as_str() {
                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                            track.cover_art = Some(bytes);
                        }
                    }
                    let _ = tx.send(track);
                }
                line.clear();
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}
