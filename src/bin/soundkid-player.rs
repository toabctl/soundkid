extern crate clap;

use clap::{Arg, App, crate_version};


fn main() {
    let matches = App::new("soundkid-player")
       .version(crate_version!())
       .about("Soundkid Spotify player")
        .author("Thomas Bechtold <thomasbechtold@jpberlin.de>")
        .arg(Arg::with_name("spotify_username")
             .required(true)
             .index(1)
             .help("The spotify username"))
        .arg(Arg::with_name("spotify_password")
             .required(true)
             .index(2)
             .help("The spotify password"))
        .arg(Arg::with_name("spotify_id")
             .required(true)
             .index(3)
             .help("The spotify-id to play"))
       .get_matches();

    let spotify_username = String::from(matches.value_of("spotify_username").unwrap());
    let spotify_password = String::from(matches.value_of("spotify_password").unwrap());
    let spotify_id = String::from(matches.value_of("spotify_id").unwrap());

    let mut player = player::SpotifyPlayer::new(spotify_username.clone(), spotify_password.clone());
    player.play(spotify_id.clone());
}


mod player {
    use log::{info,warn};
    use http::Uri;
    use tokio_core::reactor::Core;
    use librespot::core::authentication::Credentials;
    use librespot::core::config::SessionConfig;
    use librespot::core::session::Session;
    use librespot::core::spotify_id::SpotifyId;
    use librespot::playback::config::PlayerConfig;
    use librespot::playback::audio_backend;
    use librespot::playback::player::Player;
    use librespot::metadata::{Metadata, Playlist, Track, Album};

    pub struct SpotifyPlayer {
        session: Session,
        core: Core,
        player: Player,
    }

    enum SpotifyUriType {
        Track,
        Album,
        Playlist,
        Unknown,
    }
    
    impl SpotifyPlayer {
        pub fn new(username: String, password: String) -> Self {
            let mut c = Core::new().unwrap();
            let session_config = SessionConfig::default();
            let credentials = Credentials::with_password(username, password);
            let s = c.run(Session::connect(session_config, credentials, None, c.handle())).unwrap();
            let player_config = PlayerConfig::default();
            let backend = audio_backend::find(None).unwrap();
            let (p, _) = Player::new(player_config, s.clone(), None, move || {
                (backend)(None)
            });
            let sp = SpotifyPlayer {
                session: s,
                core: c,
                player: p,
            };
            return sp;
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

            warn!("Did not handle uri '{}' with path '{}'", uri, uri_new.path());
            return (uri, SpotifyUriType::Unknown);
        }

        pub fn play(&mut self, uri: String) {
            let (spotify_uri, spotify_uri_type) = self.public_uri_to_spotify_id(uri.clone());
            let spotify_id = SpotifyId::from_uri(spotify_uri.as_str()).unwrap();

            let mut tracks = Vec::new();

            // fill the list of tracks to play
            match spotify_uri_type {
                SpotifyUriType::Album => tracks = self.core.run(Album::get(&self.session.clone(), spotify_id)).unwrap().tracks,
                SpotifyUriType::Playlist => tracks = self.core.run(Playlist::get(&self.session.clone(), spotify_id)).unwrap().tracks,
                SpotifyUriType::Track => tracks = vec!(spotify_id),
                _ => warn!("uri type not handled. no tracks will be played")
            }

            for t in tracks {
                let md_track = self.core.run(Track::get(&self.session.clone(), t)).unwrap();
                info!("Playing track '{}' from {:?} ...", md_track.name, t);
                self.player.load(t, true, 0);
                let f = self.player.get_end_of_track_future();
                self.core.run(f).unwrap();
                info!("Playing spotify track {:?} finished", t);
            }
        }
    }
}
