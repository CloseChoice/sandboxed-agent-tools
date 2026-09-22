//! Earlier permission model; broader categories are not wired into the MVP.
use std::path::PathBuf;

/// Host policy categories for the WASI Preview 2 capability interfaces.
///
/// This is SAT policy, not an enum defined by the WASI specification. Each entry
/// describes the complete policy for its category. A consumer must reject duplicate
/// categories and treat omitted categories as denied, rather than merge grants.
/// `wasi:io` streams/poll/error operate on already granted resources and confer no
/// separate ambient authority. Guest exports (CLI run / HTTP incoming-handler)
/// are entry points, not permissions; the host controls when it invokes them.
///
/// These declarations do not enforce policy. Denied imports must be rejected or
/// implemented by denying host functions. WasiCtxBuilder defaults alone do not
/// deny clocks or randomness. HTTP also requires its own host implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasiP2Permissions {
    Stdin(StdinPolicy),
    Stdout(OutputPolicy),
    Stderr(OutputPolicy),
    /// Terminal handles/TTY disclosure are independent of stream access.
    Terminals(TerminalPolicy),
    /// Only explicitly supplied values; never implicitly inherit the host env.
    Environment(Grant<Vec<(String, String)>>),
    Arguments(Grant<Vec<String>>),
    /// Guest-visible directory name only; this grants no filesystem access.
    InitialCwd(Grant<String>),
    DirectoryGrants(Grant<Vec<DirectoryGrant>>),
    NetworkPolicy(NetworkPolicy),
    /// Outgoing HTTP requests, independent of raw socket authority.
    Http(Grant<Vec<HttpEndpoint>>),
    Clocks(ClockPolicy),
    Random(RandomPolicy),
    /// Guest termination only; never permission to terminate the host process.
    Exit(Access),
}

/// Deny access unless host policy explicitly approves it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Deny,
    Allow,
}

/// A complete category grant. For collections, an empty allow-list grants nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Grant<T> {
    #[default]
    Deny,
    Allow(T),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StdinPolicy {
    /// Closed input (EOF); no access to host stdin.
    #[default]
    Deny,
    /// Only these bytes are readable.
    Bytes(Vec<u8>),
    Inherit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OutputPolicy {
    /// Discard output; no access to host stdout/stderr.
    #[default]
    Deny,
    /// Host-owned buffer; fail further writes when the limit is reached.
    Capture {
        max_bytes: usize,
    },
    Inherit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalPolicy {
    pub stdin: Access,
    pub stdout: Access,
    pub stderr: Access,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockPolicy {
    pub wall_clock: Access,
    /// Includes time reads, resolution and timer subscriptions.
    pub monotonic_clock: Access,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RandomPolicy {
    pub secure: Access,
    pub insecure: Access,
    pub insecure_seed: Access,
}

/// Filesystem access allowed under a directory grant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DirectoryAccess {
    #[default]
    ReadOnly,
    ReadWrite,
    /// Separate directory enumeration/mutation from file content access.
    Custom {
        read_directory: bool,
        mutate_directory: bool,
        read_files: bool,
        write_files: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryGrant {
    pub host_path: PathBuf,
    pub guest_path: String,
    pub access: DirectoryAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

/// An outbound destination approved by host policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEndpoint {
    /// Exact DNS name or IP literal, not a URL or wildcard pattern.
    pub host: String,
    pub port: u16,
    pub method: RequestMethod,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    #[default]
    Deny,
    /// Convenience policy for outbound TCP to these destinations only.
    /// UDP, local binds/listeners and guest DNS lookup remain denied.
    /// Host-mediated resolution must pin approved addresses before connecting.
    AllowEndpoint(Vec<NetworkEndpoint>),
    Allow(NetworkGrants),
}

/// Complete raw socket policy. All lists are allow-lists; empty means denied.
///
/// Enforce both address family and operation restrictions. DNS authorization
/// never grants permission to connect to the resulting addresses. Check every
/// resolved address and every UDP datagram destination. Approved listening sockets
/// may accept connections; accepted sockets carry authority for their peer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkGrants {
    pub ipv4: Access,
    pub ipv6: Access,
    pub tcp: TcpGrants,
    pub udp: UdpGrants,
    /// Exact names the guest may resolve. Deny requires omitting or replacing
    /// name lookup; Wasmtime's global lookup toggle cannot enforce a name list.
    pub name_lookup: Grant<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TcpGrants {
    pub create_socket: Access,
    pub connect: Vec<NetworkEndpoint>,
    /// Exact local addresses, including ports; port zero permits an ephemeral port.
    pub bind: Vec<std::net::SocketAddr>,
    /// Local endpoints allowed to listen; they must also be approved for bind.
    pub listen: Vec<std::net::SocketAddr>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UdpGrants {
    pub create_socket: Access,
    pub bind: Vec<std::net::SocketAddr>,
    /// Approved peers for connected and unconnected datagrams.
    pub send_to: Vec<NetworkEndpoint>,
    /// Approved source IP addresses and ports for incoming datagrams.
    pub receive_from: Vec<std::net::SocketAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpScheme {
    Http,
    Https,
}

/// Exact outgoing HTTP origin. Raw socket grants do not authorize HTTP or vice versa.
/// The host must check every request/redirect and enforce the approved destination
/// during DNS resolution and connection establishment, including proxy use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpEndpoint {
    pub scheme: HttpScheme,
    pub endpoint: NetworkEndpoint,
}

/// WASI settings approved by host policy for one invocation.
///
/// Defaults select P2, closed stdin, discarded stdout/stderr, no environment
/// variables or directory grants, and denied network access. Clock and random
/// services are not configured by this type.
///
/// Constructing this value does not enforce permissions. The runtime must check
/// requests against host policy and apply the approved settings to WASI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasiConfig {
    pub inherit_stdin: bool,
    pub inherit_stdout: bool,
    pub inherit_stderr: bool,
    /// Explicit guest environment variables; do not inherit the host environment.
    pub env: Vec<(String, String)>,
    pub directories: Vec<DirectoryGrant>,
    pub network: NetworkPolicy,
}
