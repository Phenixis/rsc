use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use directories::UserDirs;
use rsc::backend::{Backend, LocalBackend};
use rsc::models::Playlist;
use rsc::player::{MpvOptions, MpvPlayer};
use rsc::session::{Flow, Session};
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::mpsc::Receiver;
use tokio_stream::StreamExt;

#[derive(Parser)]
#[command(version, about = "A terminal SoundCloud client")]
struct Cli {
    /// Where playlists and audio come from.
    #[arg(long, value_enum, default_value_t = BackendKind::Local, env = "RSC_BACKEND")]
    backend: BackendKind,

    /// Library folder for the local backend (default: ~/Music/rsc-dev).
    #[arg(long, env = "RSC_LOCAL_DIR", value_name = "DIR")]
    library: Option<PathBuf>,

    /// Extra argument passed to mpv, repeatable (e.g. --mpv-arg=--ao=null).
    #[arg(long = "mpv-arg", value_name = "ARG")]
    mpv_args: Vec<String>,

    /// Log more (repeat for debug output).
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendKind {
    Local,
}

#[derive(Subcommand)]
enum Command {
    /// List your playlists.
    Playlists,
    /// Play a playlist from the first track (name or id).
    Play { playlist: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    TogglePause,
    Next,
    Previous,
    Quit,
}

fn map_key(key: KeyEvent) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    match key.code {
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Char('n') | KeyCode::Right => Some(Action::Next),
        KeyCode::Char('p') | KeyCode::Left => Some(Action::Previous),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        // Raw mode turns Ctrl-C into a plain key event.
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(match cli.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            _ => tracing::Level::DEBUG,
        })
        .init();

    let backend: Arc<dyn Backend> = match cli.backend {
        BackendKind::Local => Arc::new(LocalBackend::new(library_dir(cli.library))),
    };

    match cli.command {
        Command::Playlists => {
            for p in backend.my_playlists().await? {
                println!("{}  ({} tracks)", p.title, p.track_count);
            }
            Ok(())
        }
        Command::Play { playlist } => play(backend, &playlist, cli.mpv_args).await,
    }
}

fn library_dir(explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| {
        UserDirs::new()
            .and_then(|d| d.audio_dir().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("Music"))
            .join("rsc-dev")
    })
}

/// Exact id first, then case-insensitive title.
fn find_playlist<'a>(playlists: &'a [Playlist], query: &str) -> Result<&'a Playlist> {
    playlists
        .iter()
        .find(|p| p.id.0 == query)
        .or_else(|| {
            playlists
                .iter()
                .find(|p| p.title.eq_ignore_ascii_case(query))
        })
        .with_context(|| {
            let known: Vec<_> = playlists.iter().map(|p| p.title.as_str()).collect();
            format!(
                "no playlist named {query:?} (available: {})",
                known.join(", ")
            )
        })
}

async fn play(backend: Arc<dyn Backend>, query: &str, mpv_args: Vec<String>) -> Result<()> {
    let playlists = backend.my_playlists().await?;
    let playlist = find_playlist(&playlists, query)?;
    let tracks = backend.playlist_tracks(&playlist.id).await?;
    // Registered before mpv exists, so no signal can kill us while it is running.
    let mut shutdown = ShutdownSignals::new()?;

    let (mpv, events) = MpvPlayer::spawn(MpvOptions {
        extra_args: mpv_args,
        ..MpvOptions::default()
    })
    .await?;
    let mpv = Arc::new(mpv);
    let mut session = Session::new(backend, mpv.clone(), tracks);
    if session.queue().is_empty() {
        mpv.shutdown().await;
        bail!("{:?} has no playable track", playlist.title);
    }
    if session.queue().skipped() > 0 {
        eprintln!("{} blocked track(s) skipped", session.queue().skipped());
    }

    let result = run(&mut session, events, &mut shutdown).await;
    mpv.shutdown().await;
    result
}

/// Puts the terminal in raw mode for its lifetime, restoring it on drop
/// (including when unwinding from a panic).
struct RawMode;

impl RawMode {
    fn enable() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// SIGINT, SIGTERM and SIGHUP (terminal closed) all mean "clean up and leave": dying
/// on them would orphan the mpv child and leave its socket behind.
struct ShutdownSignals {
    interrupt: Signal,
    terminate: Signal,
    hangup: Signal,
}

impl ShutdownSignals {
    fn new() -> Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
            hangup: signal(SignalKind::hangup())?,
        })
    }

    async fn recv(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
            _ = self.hangup.recv() => {}
        }
    }
}

async fn run(
    session: &mut Session,
    mut events: Receiver<rsc::player::PlayerEvent>,
    shutdown: &mut ShutdownSignals,
) -> Result<()> {
    // Without a terminal (pipes, CI) there are no keys: just play to the end.
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let _raw = if interactive {
        Some(RawMode::enable()?)
    } else {
        None
    };
    let mut keys = interactive.then(EventStream::new);

    if session.start().await? == Flow::Finished {
        bail!("no track could be played");
    }
    show_now_playing(session, interactive);
    if interactive {
        print_line("[space] pause  [n]ext  [p]revious  [q]uit", interactive);
    }

    loop {
        let before = session.queue().position();
        let flow = tokio::select! {
            event = events.recv() => match event {
                Some(event) => session.handle_event(event).await?,
                None => bail!("mpv exited unexpectedly"),
            },
            action = next_action(&mut keys) => match action {
                Action::Quit => break,
                Action::TogglePause => {
                    session.toggle_pause().await?;
                    let state = if session.is_paused() { "⏸ paused" } else { "▶ resumed" };
                    print_line(state, interactive);
                    Flow::Continue
                }
                Action::Next => session.next().await?,
                Action::Previous => session.previous().await?,
            },
            _ = shutdown.recv() => break,
        };
        if session.queue().position() != before {
            show_now_playing(session, interactive);
        }
        if flow == Flow::Finished {
            print_line("Playlist finished.", interactive);
            break;
        }
    }
    Ok(())
}

/// Resolves on the next recognised key; never resolves when there is no terminal.
async fn next_action(keys: &mut Option<EventStream>) -> Action {
    let Some(stream) = keys else {
        return std::future::pending().await;
    };
    loop {
        match stream.next().await {
            Some(Ok(Event::Key(key))) => {
                if let Some(action) = map_key(key) {
                    return action;
                }
            }
            Some(Ok(_)) => {}
            // Terminal went away.
            Some(Err(_)) | None => return Action::Quit,
        }
    }
}

fn show_now_playing(session: &Session, raw: bool) {
    if let Some(track) = session.queue().current() {
        let (n, total) = session.queue().position();
        print_line(
            &format!("▶ {n}/{total}  {} — {}", track.title, track.artist),
            raw,
        );
    }
}

/// Raw mode does not translate `\n` to `\r\n`, so we do it ourselves.
fn print_line(text: &str, raw: bool) {
    let newline = if raw { "\r\n" } else { "\n" };
    print!("{text}{newline}");
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsc::models::PlaylistId;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_map_to_actions() {
        let none = KeyModifiers::NONE;
        assert_eq!(
            map_key(key(KeyCode::Char(' '), none)),
            Some(Action::TogglePause)
        );
        assert_eq!(map_key(key(KeyCode::Char('n'), none)), Some(Action::Next));
        assert_eq!(
            map_key(key(KeyCode::Char('p'), none)),
            Some(Action::Previous)
        );
        assert_eq!(map_key(key(KeyCode::Char('q'), none)), Some(Action::Quit));
        assert_eq!(
            map_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::Quit)
        );
        assert_eq!(map_key(key(KeyCode::Char('c'), none)), None);
        assert_eq!(map_key(key(KeyCode::Char('x'), none)), None);
    }

    #[test]
    fn playlist_lookup_prefers_id_then_title_case_insensitively() {
        let playlists = vec![
            Playlist {
                id: PlaylistId("Rock".into()),
                title: "Rock".into(),
                track_count: 1,
            },
            Playlist {
                id: PlaylistId("jazz".into()),
                title: "Jazz".into(),
                track_count: 1,
            },
        ];
        assert_eq!(find_playlist(&playlists, "Rock").unwrap().title, "Rock");
        assert_eq!(find_playlist(&playlists, "JAZZ").unwrap().title, "Jazz");
        let err = find_playlist(&playlists, "metal").unwrap_err().to_string();
        assert!(err.contains("Rock, Jazz"), "{err}");
    }
}
