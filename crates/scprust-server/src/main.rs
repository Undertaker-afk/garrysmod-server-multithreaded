//! Linux-only UDP prototype. Intentionally separate from the HTTP offload daemon.
#![forbid(unsafe_code)]
use scprust_server::{
    query::ServerInfo,
    transport::{QueryEngine, Tokens},
};
use std::{
    fs::File,
    io::{self, Read, Write},
    net::{SocketAddr, UdpSocket},
    process::ExitCode,
    time::{Duration, Instant},
};

const HELP: &str = "scprust-server: query-only replacement-server transport prototype
Usage: scprust-server [--bind 127.0.0.1:27025] [--name SCPRust]
       scprust-server --help | --version
Only loopback addresses are allowed. Port 0 chooses an ephemeral UDP port.
No client sign-on, Steam authentication, map loading, or gameplay is implemented.
Do not replace your live srcds process with this executable.";

#[derive(Debug, PartialEq, Eq)]
struct Config {
    bind: SocketAddr,
    name: String,
}
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Run(Config),
    Help,
    Version,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let mut config = Config {
        bind: "127.0.0.1:27025".parse().unwrap(),
        name: "SCPRust".into(),
    };
    let (mut has_bind, mut has_name) = (false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Action::Help),
            "--version" => return Ok(Action::Version),
            "--bind" if !has_bind => {
                has_bind = true;
                config.bind = args
                    .next()
                    .ok_or("--bind requires an IP:port")?
                    .parse()
                    .map_err(|_| "--bind requires a numeric IP:port (IPv6 uses brackets)")?;
            }
            "--name" if !has_name => {
                has_name = true;
                config.name = args.next().ok_or("--name requires a label")?;
            }
            _ => return Err(format!("unknown or duplicate argument: {arg}")),
        }
    }
    if !config.bind.ip().is_loopback() {
        return Err("only loopback binds are supported in this prototype".into());
    }
    ServerInfo::new(&config.name)?;
    Ok(Action::Run(config))
}

struct OsTokens(File);
impl Tokens for OsTokens {
    fn next_token(&mut self) -> io::Result<[u8; 4]> {
        let mut token = [0; 4];
        self.0.read_exact(&mut token)?;
        Ok(token)
    }
}

fn run(config: Config) -> io::Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(io::Error::other(
            "this prototype currently targets Linux only",
        ));
    }
    let mut tokens = OsTokens(File::open("/dev/urandom")?);
    let socket = UdpSocket::bind(config.bind)?;
    let info = ServerInfo::new(&config.name).map_err(io::Error::other)?;
    let mut engine = QueryEngine::new(info);
    let started = Instant::now();
    let mut last_report = Duration::ZERO;
    let mut send_errors = 0u64;
    // Larger packets cannot be mistaken for a valid 9/25/29-byte query, even
    // when the OS truncates a datagram to this buffer's size.
    let mut buffer = [0u8; 2048];
    eprintln!(
        "QUERY ONLY: no client sign-on/authentication/gameplay; not a live srcds replacement"
    );
    println!("QUERY_ONLY_READY {}", socket.local_addr()?);
    io::stdout().flush()?;
    loop {
        let (length, peer) = match socket.recv_from(&mut buffer) {
            Ok(result) => result,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let now = started.elapsed();
        if let Some(reply) = engine.handle(peer, &buffer[..length], now, &mut tokens)? {
            if socket.send_to(&reply, peer).is_err() {
                send_errors = send_errors.saturating_add(1);
            }
        }
        if now.saturating_sub(last_report) >= Duration::from_secs(60) {
            eprintln!(
                "query counters: {:?}, send_errors={send_errors}",
                engine.counters()
            );
            last_report = now;
        }
    }
}

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)) {
        Ok(Action::Help) => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Action::Version) => {
            println!("scprust-server {} (query only)", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Action::Run(config)) => match run(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("server error: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(args: &[&str]) -> Result<Action, String> {
        parse(args.iter().map(|s| (*s).to_string()))
    }
    #[test]
    fn defaults_are_separate_from_live_server_ports() {
        assert_eq!(
            args(&[]).unwrap(),
            Action::Run(Config {
                bind: "127.0.0.1:27025".parse().unwrap(),
                name: "SCPRust".into()
            })
        );
        assert!(args(&["--bind", "[::1]:0", "--name", "test"]).is_ok());
        assert_eq!(args(&["--help"]).unwrap(), Action::Help);
    }
    #[test]
    fn rejects_public_and_ambiguous_settings() {
        for args_ in [
            vec!["--bind", "0.0.0.0:27015"],
            vec!["--bind", "[::]:27015"],
            vec!["--bind", "localhost:27015"],
            vec!["--bind"],
            vec!["--name"],
            vec!["--name", "bad\0name"],
            vec!["--name", "a", "--name", "b"],
            vec!["--bogus"],
        ] {
            assert!(args(&args_).is_err(), "unexpectedly accepted: {args_:?}");
        }
    }
}
