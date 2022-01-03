use http::Uri;
use librespot::core::authentication::Credentials;
use librespot::core::config::SessionConfig;
use librespot::core::session::Session;
use librespot::core::spotify_id::SpotifyId;
use librespot::metadata::{Album, Metadata, Playlist, Track};
use librespot::playback::audio_backend;
use librespot::playback::config::{AudioFormat, PlayerConfig};
use librespot::playback::player::Player;
use log::{info, warn};

pub struct SpotifyPlayer {
    session: Session,
    player: Player,
}

enum SpotifyUriType {
    Track,
    Album,
    Playlist,
    Unknown,
}

impl SpotifyPlayer {
    pub async fn new(username: String, password: String) -> Self {
        let session_config = SessionConfig::default();
        let player_config = PlayerConfig::default();
        let audio_format = AudioFormat::default();
        let credentials = Credentials::with_password(username, password);
        let backend = audio_backend::find(None).unwrap();

        println!("Connecting ..");
        let s = Session::connect(session_config, credentials, None)
            .await
            .unwrap();

        let (p, _) = Player::new(player_config, s.clone(), None, move || {
            backend(None, audio_format)
        });

        let sp = SpotifyPlayer {
            session: s,
            player: p,
        };
        return sp;
    }

    pub async fn play(&mut self, uri: String) {
        let (spotify_uri, spotify_uri_type) = self.public_uri_to_spotify_id(uri.clone());
        let spotify_id = SpotifyId::from_uri(spotify_uri.as_str()).unwrap();

        let mut tracks = Vec::new();

        // fill the list of tracks to play
        match spotify_uri_type {
            SpotifyUriType::Album => {
                tracks = Album::get(&self.session.clone(), spotify_id)
                    .await
                    .unwrap()
                    .tracks
            }
            SpotifyUriType::Playlist => {
                tracks = Playlist::get(&self.session.clone(), spotify_id)
                    .await
                    .unwrap()
                    .tracks
            }
            SpotifyUriType::Track => tracks = vec![spotify_id],
            _ => warn!("uri type not handled. no tracks will be played"),
        }

        for t in tracks {
            let md_track = Track::get(&self.session.clone(), t).await.unwrap();
            info!("Playing track '{}' from {:?} ...", md_track.name, t);
            self.player.load(t, true, 0);
            self.player.play();
            //self.player.await_end_of_track().await;
            info!("Playing spotify track {:?} finished", t);
        }
    }

    pub async fn stop(&mut self) {
        self.player.stop();
        info!("Playing stopped ...");
    }

    pub async fn pause(&mut self) {
        self.player.pause();
        info!("Playing paused ...");
    }

    pub async fn resume(&mut self) {
        self.player.play();
        info!("Playing resumed ...");
    }

    fn public_uri_to_spotify_id(&self, uri: String) -> (String, SpotifyUriType) {
        // 1) maybe it's already something that we understand
        if uri.starts_with("spotify:album:") {
            return (uri.clone(), SpotifyUriType::Album);
        } else if uri.starts_with("spotify:track:") {
            return (uri.clone(), SpotifyUriType::Track);
        } else if uri.starts_with("spotify:playlist:") {
            return (uri.clone(), SpotifyUriType::Playlist);
        }

        // 2) try to parse the uri
        let uri_new = uri.parse::<Uri>().unwrap();
        if uri_new.path().starts_with("/track/") {
            let mut s = String::from("spotify:track:");
            s.push_str(&uri_new.path().replace("/track/", ""));
            return (s, SpotifyUriType::Track);
        } else if uri_new.path().starts_with("/album/") {
            let mut s = String::from("spotify:album:");
            s.push_str(&uri_new.path().replace("/album/", ""));
            return (s, SpotifyUriType::Album);
        } else if uri_new.path().starts_with("/playlist/") {
            let mut s = String::from("spotify:playlist:");
            s.push_str(&uri_new.path().replace("/playlist/", ""));
            return (s, SpotifyUriType::Playlist);
        }

        warn!(
            "Did not handle uri '{}' with path '{}'",
            uri,
            uri_new.path()
        );
        return (uri, SpotifyUriType::Unknown);
    }
}
