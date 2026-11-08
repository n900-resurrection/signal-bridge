//! signal-bridge: presage (Signal) behind a local unix socket with a simple line protocol.
//!
//! It opens the presage store of an already linked device (the one of gurk-rs, see `--store` and `--gurk-config`), receives
//! messages and offers sending, the contact list and the group list to local clients (the libpurple plugin for the Maemo
//! messaging UI). One line per message, fields separated by TAB; `\\`, `\t` and `\n` inside a field are escaped.
//!
//! Client to bridge (`ID` is any token chosen by the client, echoed in the answer):
//!   SEND  ID  UUID  TEXT          send a text to a contact
//!   GSEND ID  KEY   TEXT          send a text to a group (KEY = 64 hex digits, the group master key)
//!   CONTACTS                      list the contacts   (answer: CONTACT lines, then CONTACTS-END)
//!   GROUPS                        list the groups     (answer: GROUP lines, then GROUPS-END)
//!   HISTORY ID THREAD SINCE       stored messages of a chat (THREAD = contact UUID or group key, SINCE = milliseconds,
//!                                 0 = all) as MSG/GMSG/SENT/GSENT lines, then HISTORY-END ID. A client that was not
//!                                 connected when a message arrived asks for it this way (the bridge does not drop it:
//!                                 presage stores every received message).
//!   PING                          answer: PONG
//! Bridge to client:
//!   READY  UUID  PHONE            sent on connect: our own identity
//!   OK  ID  TIMESTAMP | ERR  ID  REASON
//!   CONTACT  UUID  NAME  PHONE | GROUP  KEY  TITLE
//!   MSG   UUID  TIMESTAMP  TEXT   received in a chat with a contact (UUID = the sender)
//!   GMSG  KEY  UUID  TIMESTAMP  TEXT   received in a group
//!   SENT  UUID  TIMESTAMP  TEXT   written on another device of ours to a contact
//!   GSENT KEY   TIMESTAMP  TEXT   written on another device of ours to a group
//!
//! Safety for tests: with `--allow UUID,...` (or SIGNAL_BRIDGE_ALLOW) only these recipients may be written to.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context as _};
use clap::{Parser, Subcommand};
use futures::{pin_mut, StreamExt};
use presage::libsignal_service::content::{Content, ContentBody, DataMessage, GroupContextV2};
use presage::libsignal_service::prelude::Uuid;
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::zkgroup::GroupMasterKeyBytes;
use presage::manager::Registered;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::proto::sync_message::{Content as SyncContent, Sent};
use presage::proto::SyncMessage;
use presage::store::{ContentsStore, Thread};
use presage::Manager;
use presage_store_sqlite::SqliteStore;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

type Mgr = Manager<SqliteStore, Registered>;

#[derive(Parser)]
#[command(version, about = "Signal daemon for the N900 (presage behind a local socket)")]
struct Args {
    /// presage store (SQLite, sqlcipher); default: the store of gurk-rs
    #[arg(long, env = "SIGNAL_BRIDGE_STORE")]
    store: Option<PathBuf>,
    /// gurk.toml to read `passphrase = "..."` from (the store is encrypted with it)
    #[arg(long, env = "SIGNAL_BRIDGE_GURK_CONFIG")]
    gurk_config: Option<PathBuf>,
    /// recipients (UUIDs, comma separated) that may be written to; without it everybody
    #[arg(long, env = "SIGNAL_BRIDGE_ALLOW", value_delimiter = ',')]
    allow: Vec<Uuid>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Clone)]
enum Cmd {
    /// run the daemon: receive messages and serve the socket
    Daemon {
        /// socket path; default ~/.local/share/signal-bridge/bridge.sock
        #[arg(long, env = "SIGNAL_BRIDGE_SOCKET")]
        socket: Option<PathBuf>,
    },
    /// print the contacts (UUID, name, phone) from the store; no network
    Contacts,
    /// print the groups (key, title) from the store; no network
    Groups,
    /// print the stored messages of a chat as protocol lines; no network
    Messages {
        /// contact UUID or 64 digit group key
        thread: String,
        /// only messages newer than this (milliseconds since 1970)
        #[arg(long, default_value_t = 0)]
        since: u64,
    },
    /// print who we are (UUID, phone) from the store; no network
    Whoami,
}

fn home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| anyhow!("HOME is not set"))
}

fn passphrase(args: &Args) -> anyhow::Result<Option<String>> {
    let path = match &args.gurk_config {
        Some(p) => p.clone(),
        None => home()?.join(".config/gurk/gurk.toml"),
    };
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("passphrase") {
            let rest = rest.trim_start().strip_prefix('=').map(str::trim);
            if let Some(v) = rest.and_then(|v| v.strip_prefix('"')).and_then(|v| v.strip_suffix('"')) {
                return Ok(Some(v.to_string()));
            }
        }
    }
    Ok(None)
}

async fn open_store(args: &Args) -> anyhow::Result<SqliteStore> {
    let path = match &args.store {
        Some(p) => p.clone(),
        None => home()?.join(".local/share/gurk/signal.sqlite"),
    };
    if !path.exists() {
        bail!("the store {} does not exist (link the device with gurk first)", path.display());
    }
    let pass = passphrase(args)?;
    info!(store = %path.display(), encrypted = pass.is_some(), "opening the store");
    SqliteStore::open_with_passphrase(path.to_str().context("store path")?, pass.as_deref(), OnNewIdentity::Trust)
        .await
        .context("cannot open the store (wrong passphrase?)")
}

// ---- line protocol -------------------------------------------------------------------------------------------------

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('\\') => o.push('\\'),
            Some(x) => {
                o.push('\\');
                o.push(x)
            }
            None => o.push('\\'),
        }
    }
    o
}

fn line(fields: &[&str]) -> String {
    let mut s = fields.iter().map(|f| esc(f)).collect::<Vec<_>>().join("\t");
    s.push('\n');
    s
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn parse_key(hex_key: &str) -> anyhow::Result<GroupMasterKeyBytes> {
    let v = hex::decode(hex_key).context("the group key is not hexadecimal")?;
    v.try_into().map_err(|_| anyhow!("the group key must have 64 hex digits"))
}

// ---- contacts, groups, identity ----------------------------------------------------------------------------------

async fn contact_lines(m: &Mgr) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for c in m.store().contacts().await.map_err(|e| anyhow!("{e}"))?.flatten() {
        let phone = c.phone_number.map(|p| p.to_string()).unwrap_or_default();
        out.push(line(&["CONTACT", &c.uuid.to_string(), &c.name, &phone]));
    }
    Ok(out)
}

async fn group_lines(m: &Mgr) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for g in m.store().groups().await.map_err(|e| anyhow!("{e}"))?.flatten() {
        let (key, group) = g;
        out.push(line(&["GROUP", &hex::encode(key), &group.title]));
    }
    Ok(out)
}

fn ready_line(m: &Mgr) -> String {
    let d = m.registration_data();
    line(&["READY", &d.service_ids.aci.to_string(), &d.phone_number.to_string()])
}

fn parse_thread(t: &str) -> anyhow::Result<Thread> {
    if t.len() == 64 {
        Ok(Thread::Group(parse_key(t)?))
    } else {
        let uuid: Uuid = t.parse().context("the thread is neither a UUID nor a 64 digit group key")?;
        Ok(Thread::Contact(ServiceId::Aci(uuid.into())))
    }
}

async fn history_lines(m: &Mgr, thread: &str, since: u64) -> anyhow::Result<Vec<String>> {
    let thread = parse_thread(thread)?;
    let mut out = Vec::new();
    for c in m.store().messages(&thread, since..).await.map_err(|e| anyhow!("{e}"))?.flatten() {
        if let Some(l) = incoming_line(&c) {
            out.push(l);
        }
    }
    Ok(out)
}

// ---- incoming messages -------------------------------------------------------------------------------------------

fn text_of(d: &DataMessage) -> Option<String> {
    let mut t = d.body.clone().unwrap_or_default();
    if !d.attachments.is_empty() {
        if !t.is_empty() {
            t.push(' ');
        }
        t.push_str(&format!("[{} attachment(s)]", d.attachments.len()));
    }
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// The protocol line for a received message, if it is a text message we show.
fn incoming_line(content: &Content) -> Option<String> {
    let thread = Thread::try_from(content).ok()?;
    let ts = content.metadata.client_timestamp.timestamp_millis().to_string();
    let sender = content.metadata.sender.raw_uuid().to_string();
    match (&content.body, thread) {
        (ContentBody::DataMessage(d), Thread::Contact(_)) => Some(line(&["MSG", &sender, &ts, &text_of(d)?])),
        (ContentBody::DataMessage(d), Thread::Group(key)) => Some(line(&["GMSG", &hex::encode(key), &sender, &ts, &text_of(d)?])),
        (
            ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(Sent { message: Some(d), .. })), .. }),
            Thread::Contact(to),
        ) => Some(line(&["SENT", &to.raw_uuid().to_string(), &ts, &text_of(d)?])),
        (
            ContentBody::SynchronizeMessage(SyncMessage { content: Some(SyncContent::Sent(Sent { message: Some(d), .. })), .. }),
            Thread::Group(key),
        ) => Some(line(&["GSENT", &hex::encode(key), &ts, &text_of(d)?])),
        _ => None,
    }
}

async fn receive_loop(mut manager: Mgr, tx: broadcast::Sender<String>) {
    loop {
        match manager.receive_messages().await {
            Err(e) => error!("cannot start receiving: {e}"),
            Ok(stream) => {
                info!("receiving messages");
                pin_mut!(stream);
                while let Some(received) = stream.next().await {
                    match received {
                        Received::QueueEmpty => debug!("queue empty"),
                        Received::Contacts => {
                            let _ = tx.send(line(&["CONTACTS-CHANGED"]));
                        }
                        Received::Content(c) => {
                            if let Some(l) = incoming_line(&c) {
                                let _ = tx.send(l);
                            }
                        }
                        Received::DecryptionError(e) => warn!("decryption error: {e:?}"),
                    }
                }
                warn!("the message stream ended");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

// ---- clients ---------------------------------------------------------------------------------------------------

struct Shared {
    manager: Mgr,
    allow: HashSet<Uuid>,
    tx: broadcast::Sender<String>,
}

async fn send_text(sh: &Shared, to: &str, group: bool, text: String) -> anyhow::Result<u64> {
    let ts = now_ms();
    let mut manager = sh.manager.clone();
    let mut data = DataMessage { body: Some(text), timestamp: Some(ts), ..Default::default() };
    if group {
        let key = parse_key(to)?;
        data.group_v2 = Some(GroupContextV2 { master_key: Some(key.to_vec()), revision: Some(0), ..Default::default() });
        manager.send_message_to_group(&key, ContentBody::DataMessage(data), ts).await.map_err(|e| anyhow!("{e}"))?;
    } else {
        let uuid: Uuid = to.parse().context("the recipient is not a UUID")?;
        if !sh.allow.is_empty() && !sh.allow.contains(&uuid) {
            bail!("recipient not allowed (--allow)");
        }
        manager
            .send_message(ServiceId::Aci(uuid.into()), ContentBody::DataMessage(data), ts)
            .await
            .map_err(|e| anyhow!("{e}"))?;
    }
    Ok(ts)
}

async fn handle_line(sh: &Shared, l: &str) -> Vec<String> {
    let f: Vec<String> = l.trim_end_matches(['\r', '\n']).split('\t').map(unesc).collect();
    match f.first().map(String::as_str) {
        Some("PING") => vec![line(&["PONG"])],
        Some("CONTACTS") => {
            let mut v = contact_lines(&sh.manager).await.unwrap_or_default();
            v.push(line(&["CONTACTS-END"]));
            v
        }
        Some("GROUPS") => {
            let mut v = group_lines(&sh.manager).await.unwrap_or_default();
            v.push(line(&["GROUPS-END"]));
            v
        }
        Some("HISTORY") if f.len() >= 4 => {
            let id = f[1].clone();
            match history_lines(&sh.manager, &f[2], f[3].parse().unwrap_or(0)).await {
                Ok(mut v) => {
                    v.push(line(&["HISTORY-END", &id]));
                    v
                }
                Err(e) => vec![line(&["ERR", &id, &format!("{e:#}")])],
            }
        }
        Some(cmd @ ("SEND" | "GSEND")) if f.len() >= 4 => {
            let id = f[1].clone();
            info!(%cmd, %id, "sending");
            match send_text(sh, &f[2], cmd == "GSEND", f[3..].join("\t")).await {
                Ok(ts) => {
                    info!(%cmd, %id, "sent");
                    vec![line(&["OK", &id, &ts.to_string()])]
                }
                Err(e) => {
                    warn!("{cmd} failed: {e:#}");
                    vec![line(&["ERR", &id, &format!("{e:#}")])]
                }
            }
        }
        _ => vec![line(&["ERR", "", "unknown or incomplete command"])],
    }
}

async fn client(sh: Rc<Shared>, stream: UnixStream) {
    let mut rx = sh.tx.subscribe();
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    if w.write_all(ready_line(&sh.manager).as_bytes()).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            l = lines.next_line() => match l {
                Ok(Some(l)) => for out in handle_line(&sh, &l).await {
                    if w.write_all(out.as_bytes()).await.is_err() { return; }
                },
                _ => return,
            },
            m = rx.recv() => match m {
                Ok(m) => if w.write_all(m.as_bytes()).await.is_err() { return; },
                Err(broadcast::error::RecvError::Lagged(n)) => warn!("a client lost {n} messages"),
                Err(_) => return,
            },
        }
    }
}

async fn daemon(args: &Args, socket: Option<PathBuf>) -> anyhow::Result<()> {
    let sock = match socket {
        Some(p) => p,
        None => home()?.join(".local/share/signal-bridge/bridge.sock"),
    };
    // a second daemon would take the socket over and fight for the Signal connection: refuse to start (before the slow opening of the store)
    if UnixStream::connect(&sock).await.is_ok() {
        bail!("signal-bridge is already running ({})", sock.display());
    }
    let manager = Manager::load_registered(open_store(args).await?).await.context("the store has no linked device")?;
    if let Some(dir) = sock.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).with_context(|| format!("cannot bind {}", sock.display()))?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    info!(socket = %sock.display(), "listening");

    let (tx, _) = broadcast::channel(256);
    let shared = Rc::new(Shared { manager: manager.clone(), allow: args.allow.iter().copied().collect(), tx: tx.clone() });
    tokio::task::spawn_local(receive_loop(manager, tx));
    loop {
        let (stream, _) = listener.accept().await?;
        debug!("client connected");
        tokio::task::spawn_local(client(shared.clone(), stream));
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "signal_bridge=info,sqlx=error,warn".into()))
        .init();
    let args = Args::parse();
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            match &args.command {
                Cmd::Daemon { socket } => daemon(&args, socket.clone()).await,
                Cmd::Contacts | Cmd::Groups | Cmd::Whoami | Cmd::Messages { .. } => {
                    let m = Manager::load_registered(open_store(&args).await?).await.context("the store has no linked device")?;
                    let lines = match args.command.clone() {
                        Cmd::Contacts => contact_lines(&m).await?,
                        Cmd::Groups => group_lines(&m).await?,
                        Cmd::Messages { thread, since } => history_lines(&m, &thread, since).await?,
                        _ => vec![ready_line(&m)],
                    };
                    for l in lines {
                        print!("{l}");
                    }
                    Ok(())
                }
            }
        })
        .await
}
