//! The client end, used by the desktop app and the `tether` CLI.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tether_os::local::{self, Endpoint, Stream};
use tokio::time::timeout;

use crate::auth::{client_handshake, Token};
use crate::frame::{recv, send};
use crate::proto::{Event, Reply, Request, Response, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
use crate::token;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Joining tries each nearby device in turn, up to 30 s each; leave room.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub struct Client {
    stream: Stream,
}

impl Client {
    /// Connect to the daemon running for `profile`, using its stored token.
    pub async fn connect(profile: &str) -> Result<Self> {
        let endpoint = Endpoint::for_profile(profile).context("finding the control endpoint")?;
        let token = token::load(profile)?;
        Self::connect_to(&endpoint, &token).await
    }

    /// Connect to `endpoint` and authenticate both ends with `token`.
    pub async fn connect_to(endpoint: &Endpoint, token: &Token) -> Result<Self> {
        let mut stream = timeout(CONNECT_TIMEOUT, local::connect(endpoint))
            .await
            .map_err(|_| anyhow!("timed out connecting to tetherd"))?
            .with_context(|| format!("connecting to {endpoint} (is tetherd running?)"))?;
        timeout(CONNECT_TIMEOUT, client_handshake(&mut stream, token))
            .await
            .map_err(|_| anyhow!("timed out authenticating with tetherd"))??;
        Ok(Self { stream })
    }

    /// Send one request and wait for its reply. A daemon-side error becomes `Err`.
    pub async fn request(&mut self, req: &Request) -> Result<Reply> {
        match self.call(req).await? {
            Response::Ok(reply) => Ok(reply),
            Response::Err(msg) => Err(anyhow!(msg)),
        }
    }

    /// Send one request and wait for its response. `Err` means the connection
    /// failed and is no longer usable; a refusal by the daemon is `Ok(Response::Err)`.
    pub async fn call(&mut self, req: &Request) -> Result<Response> {
        let exchange = async {
            send(&mut self.stream, req, MAX_REQUEST_BYTES).await?;
            recv::<Response, _>(&mut self.stream, MAX_REPLY_BYTES).await
        };
        Ok(timeout(REQUEST_TIMEOUT, exchange).await.map_err(|_| anyhow!("tetherd did not answer"))??)
    }

    /// Turn this connection into a stream of events.
    pub async fn subscribe(mut self) -> Result<Events> {
        match self.request(&Request::Subscribe).await? {
            Reply::Subscribed => Ok(Events { stream: self.stream }),
            other => bail!("unexpected reply to Subscribe: {other:?}"),
        }
    }
}

/// A raw connection before authentication. For tests of the daemon's defences.
#[doc(hidden)]
pub async fn client_stream(endpoint: &Endpoint) -> std::io::Result<Stream> {
    local::connect(endpoint).await
}

/// A raw connection after authentication, without request framing. For tests.
#[doc(hidden)]
pub async fn authenticated_stream(endpoint: &Endpoint, token: &Token) -> Result<Stream> {
    Ok(Client::connect_to(endpoint, token).await?.stream)
}

pub struct Events {
    stream: Stream,
}

impl Events {
    /// The next event; an error means the daemon went away.
    pub async fn next(&mut self) -> Result<Event> {
        Ok(recv(&mut self.stream, MAX_REPLY_BYTES).await?)
    }
}
