use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;
use clap::{Parser, Subcommand};
use std::process::Command;
use std::fs::OpenOptions;
use std::io::Write;

// ── Constants ────────────────────────────────────────────────────────────────

const BUFLEN: usize = 4096 * 4;
const TIMEOUT_SECS: u64 = 60;
const IDLE_CHECK_SECS: u64 = 3;
const DEFAULT_HOST: &str = "127.0.0.1:111";
const RESPONSE: &str = concat!(
    "HTTP/1.1 101 <b><i><font color=\"blue\">RYOTWELL.VERCEL.APP</font></b> Switching Protocols\r\n",
    "Upgrade: websocket\r\n",
    "Connection: Upgrade\r\n",
    "Sec-WebSocket-Accept: foo\r\n",
    "\r\n"
);
const SOCKS_VERSION: u8 = 5;

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "proxy", about = "Dual-protocol HTTP CONNECT / SOCKS5 proxy")]
struct Args {
    /// Bind address
    #[arg(short = 'b', long = "bind", default_value = "127.0.0.1")]
    bind: String,

    /// Listening port
    #[arg(short = 'p', long = "port", default_value_t = 700)]
    port: u16,

    /// Optional password (X-Pass header for HTTP mode)
    #[arg(long = "pass", default_value = "")]
    pass: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Add a system user for SSH tunnel (no shell, no home)
    AddUser {
        /// Username
        username: String,
        /// Password
        password: String,
    },
    /// Delete a system user
    DelUser {
        /// Username
        username: String,
    },
    /// Configure sshd with hardened settings for tunnel users
    SetupSshd,
}

// ── Server ───────────────────────────────────────────────────────────────────

struct Server {
    addr: String,
    port: u16,
    pass: Arc<String>,
    /// Active connection count (for logging / future use)
    conn_count: Arc<Mutex<usize>>,
}

impl Server {
    fn new(addr: &str, port: u16, pass: &str) -> Self {
        Self {
            addr: addr.to_owned(),
            port,
            pass: Arc::new(pass.to_owned()),
            conn_count: Arc::new(Mutex::new(0)),
        }
    }

    async fn run(&self) -> io::Result<()> {
        let bind_addr = format!("{}:{}", self.addr, self.port);
        let listener = TcpListener::bind(&bind_addr).await?;

        println!("\n:-------RustProxy-------:\n");
        println!("Listening addr: {}", self.addr);
        println!("Listening port: {}\n", self.port);
        println!(":------------------------:\n");

        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    let pass = Arc::clone(&self.pass);
                    let count = Arc::clone(&self.conn_count);
                    {
                        let mut c = count.lock().await;
                        *c += 1;
                    }
                    tokio::spawn(async move {
                        handle_connection(socket, addr, pass).await;
                        let mut c = count.lock().await;
                        *c -= 1;
                    });
                }
                Err(e) => {
                    eprintln!("Accept error: {e}");
                }
            }
        }
    }
}

// ── Connection handler ───────────────────────────────────────────────────────

async fn handle_connection(mut client: TcpStream, addr: SocketAddr, pass: Arc<String>) {
    let mut log = format!("Connection: {addr}");
    let mut buf = vec![0u8; BUFLEN];

    let n = match client.read(&mut buf).await {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let initial = buf[..n].to_vec();

    let result = if initial[0] == SOCKS_VERSION {
        handle_socks5(&mut client, &mut log, initial).await
    } else {
        handle_http(&mut client, &mut log, initial, &pass).await
    };

    if let Err(e) = result {
        log.push_str(&format!(" - error: {e}"));
        eprintln!("{log}");
    }
}

// ── HTTP CONNECT handler ─────────────────────────────────────────────────────

async fn handle_http(
    client: &mut TcpStream,
    log: &mut String,
    initial: Vec<u8>,
    pass: &str,
) -> io::Result<()> {
    let headers = String::from_utf8_lossy(&initial).into_owned();

    let host_port = find_header(&headers, "X-Real-Host")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_HOST.to_owned());

    // X-Split: consume an extra buffer worth of data before proxying
    if find_header(&headers, "X-Split").map_or(false, |v| !v.is_empty()) {
        let mut tmp = vec![0u8; BUFLEN];
        let _ = client.read(&mut tmp).await;
    }

    let passwd = find_header(&headers, "X-Pass").unwrap_or_default();

    if !pass.is_empty() && passwd != pass {
        client
            .write_all(b"HTTP/1.1 400 WrongPass!\r\n\r\n")
            .await?;
        return Ok(());
    }

    if !pass.is_empty()
        || host_port.starts_with("127.0.0.1")
        || host_port.starts_with("localhost")
    {
        method_connect(client, log, &host_port).await
    } else {
        client
            .write_all(b"HTTP/1.1 403 Forbidden!\r\n\r\n")
            .await?;
        Ok(())
    }
}

async fn method_connect(
    client: &mut TcpStream,
    log: &mut String,
    path: &str,
) -> io::Result<()> {
    log.push_str(&format!(" - CONNECT {path}"));

    let mut target = match connect_target(path).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error connecting to target {path} - {e}");
            client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
                .await?;
            return Ok(());
        }
    };

    client.write_all(RESPONSE.as_bytes()).await?;
    println!("{log}");

    do_connect(client, &mut target).await
}

// ── SOCKS5 handler ───────────────────────────────────────────────────────────

async fn handle_socks5(
    client: &mut TcpStream,
    log: &mut String,
    _initial: Vec<u8>,
) -> io::Result<()> {
    // No-auth handshake reply
    client.write_all(&[0x05, 0x00]).await?;

    // Read request header: VER CMD RSV ATYP
    let mut req_hdr = [0u8; 4];
    client.read_exact(&mut req_hdr).await?;
    let (_version, cmd, _rsv, address_type) = (req_hdr[0], req_hdr[1], req_hdr[2], req_hdr[3]);

    if cmd != 1 {
        // Only CONNECT (0x01) supported
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Only SOCKS5 CONNECT is supported",
        ));
    }

    let address = match address_type {
        1 => {
            // IPv4
            let mut ipv4 = [0u8; 4];
            client.read_exact(&mut ipv4).await?;
            format!("{}.{}.{}.{}", ipv4[0], ipv4[1], ipv4[2], ipv4[3])
        }
        3 => {
            // Domain name
            let mut len_buf = [0u8; 1];
            client.read_exact(&mut len_buf).await?;
            let domain_len = len_buf[0] as usize;
            let mut domain_buf = vec![0u8; domain_len];
            client.read_exact(&mut domain_buf).await?;
            String::from_utf8_lossy(&domain_buf).into_owned()
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Unsupported SOCKS5 address type: {address_type}"),
            ));
        }
    };

    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    log.push_str(&format!(" - SOCKS5 CONNECT {address}:{port}"));

    let target_addr = format!("{address}:{port}");
    let mut target = match connect_target(&target_addr).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error connecting to target {target_addr} - {e}");
            return Err(e);
        }
    };

    // Send success reply
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .await?;

    println!("{log}");
    do_connect(client, &mut target).await
}

// ── Bidirectional tunnel ─────────────────────────────────────────────────────

/// Bidirectional copy with an inactivity timeout.
/// Mirrors the Python `doCONNECT` logic: count idle 3-second ticks and break
/// when `count >= TIMEOUT` (i.e. 60 ticks × 3 s = 180 s idle).
async fn do_connect(client: &mut TcpStream, target: &mut TcpStream) -> io::Result<()> {
    let (mut cr, mut cw) = client.split();
    let (mut tr, mut tw) = target.split();

    let client_to_target = async {
        let mut buf = vec![0u8; BUFLEN];
        let mut idle_ticks: u64 = 0;
        loop {
            match timeout(Duration::from_secs(IDLE_CHECK_SECS), cr.read(&mut buf)).await {
                Ok(Ok(0)) | Err(_) => {
                    idle_ticks += 1;
                    if idle_ticks >= TIMEOUT_SECS / IDLE_CHECK_SECS {
                        break;
                    }
                }
                Ok(Ok(n)) => {
                    idle_ticks = 0;
                    if let Err(e) = tw.write_all(&buf[..n]).await {
                        eprintln!("Data transfer error (c→t): {e}");
                        break;
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Data transfer error (c→t read): {e}");
                    break;
                }
            }
        }
    };

    let target_to_client = async {
        let mut buf = vec![0u8; BUFLEN];
        let mut idle_ticks: u64 = 0;
        loop {
            match timeout(Duration::from_secs(IDLE_CHECK_SECS), tr.read(&mut buf)).await {
                Ok(Ok(0)) | Err(_) => {
                    idle_ticks += 1;
                    if idle_ticks >= TIMEOUT_SECS / IDLE_CHECK_SECS {
                        break;
                    }
                }
                Ok(Ok(n)) => {
                    idle_ticks = 0;
                    if let Err(e) = cw.write_all(&buf[..n]).await {
                        eprintln!("Data transfer error (t→c): {e}");
                        break;
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Data transfer error (t→c read): {e}");
                    break;
                }
            }
        }
    };

    // Race both directions; whichever finishes first (EOF / timeout / error)
    // effectively tears down the tunnel.
    tokio::select! {
        _ = client_to_target => {}
        _ = target_to_client => {}
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Connect to `host:port` (DNS resolved by Tokio).
async fn connect_target(host: &str) -> io::Result<TcpStream> {
    // Split on last ':' to handle IPv6 addresses gracefully
    let (h, p) = if let Some(pos) = host.rfind(':') {
        let port: u16 = host[pos + 1..]
            .parse()
            .unwrap_or(443);
        (&host[..pos], port)
    } else {
        (host, 443u16)
    };
    TcpStream::connect(format!("{h}:{p}")).await
}

/// Return the value of an HTTP header line (case-sensitive name match).
fn find_header(headers: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}: ");
    for line in headers.split("\r\n") {
        if let Some(val) = line.strip_prefix(&prefix) {
            return Some(val.to_owned());
        }
    }
    None
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if let Some(cmd) = args.command {
        match cmd {
            Commands::AddUser { username, password } => add_user(&username, &password),
            Commands::DelUser { username } => del_user(&username),
            Commands::SetupSshd => setup_sshd(),
        }
        return;
    }

    let server = Server::new(&args.bind, args.port, &args.pass);
    if let Err(e) = server.run().await {
        eprintln!("Fatal server error: {e}");
        std::process::exit(1);
    }
}

// ── Admin Helpers ────────────────────────────────────────────────────────────

fn add_user(username: &str, password: &str) {
    let _ = Command::new("groupadd").arg("-f").arg("tunnelusers").output();
    let output = Command::new("useradd")
        .arg("-M")
        .arg("-s")
        .arg("/bin/false")
        .arg("-G")
        .arg("tunnelusers")
        .arg(username)
        .output()
        .expect("Failed to execute useradd");

    if !output.status.success() {
        eprintln!("Failed to add user: {}", String::from_utf8_lossy(&output.stderr));
        return;
    }

    let mut child = Command::new("chpasswd")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to execute chpasswd");

    if let Some(mut stdin) = child.stdin.take() {
        let creds = format!("{}:{}", username, password);
        stdin.write_all(creds.as_bytes()).expect("Failed to write to stdin");
    }

    let status = child.wait().expect("Failed to wait on chpasswd");
    if status.success() {
        println!("User '{}' added successfully.", username);
    } else {
        eprintln!("Failed to set password for user '{}'.", username);
    }
}

fn del_user(username: &str) {
    let output = Command::new("userdel")
        .arg(username)
        .output()
        .expect("Failed to execute userdel");
    if output.status.success() {
        println!("User '{}' deleted successfully.", username);
    } else {
        eprintln!("Failed to delete user: {}", String::from_utf8_lossy(&output.stderr));
    }
}

fn setup_sshd() {
    let sshd_config_path = "/etc/ssh/sshd_config";
    let hardened_config = "\n# Hardened SSH settings for tunnel users
Match Group tunnelusers
    AllowTcpForwarding yes
    X11Forwarding no
    PermitTunnel no
    GatewayPorts yes
    AllowAgentForwarding no
    PermitTTY no\n";
    
    let group_output = Command::new("groupadd").arg("-f").arg("tunnelusers").output().expect("Failed to execute groupadd");
    if !group_output.status.success() {
        eprintln!("Failed to add group tunnelusers: {}", String::from_utf8_lossy(&group_output.stderr));
    }
    
    let mut file = OpenOptions::new().append(true).open(sshd_config_path).expect("Failed to open sshd_config");
    if let Err(e) = write!(file, "{}", hardened_config) {
        eprintln!("Failed to write to sshd_config: {}", e);
    } else {
        println!("sshd_config updated successfully.");
    }
    
    let restart_output = Command::new("systemctl").arg("restart").arg("sshd").output().expect("Failed to execute systemctl restart sshd");
    if restart_output.status.success() {
        println!("sshd restarted successfully.");
    } else {
        eprintln!("Failed to restart sshd: {}", String::from_utf8_lossy(&restart_output.stderr));
    }
}