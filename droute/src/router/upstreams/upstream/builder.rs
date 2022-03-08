// Copyright 2020 LEXUGE
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

#[cfg(any(feature = "doh-rustls", feature = "doh-native-tls"))]
use super::qhandle::https::Https;
use super::{
    qhandle::{udp::Udp, ConnPool, Result},
    QHandleError, Upstream,
};
use crate::{AsyncTryInto, Label};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
#[cfg(any(feature = "doh-rustls", feature = "doh-native-tls"))]
use std::net::IpAddr;
use std::{net::SocketAddr, sync::Arc, time::Duration};

// Default value for timeout
fn default_timeout() -> u64 {
    5
}

/// A builder for hybrid upstream
#[derive(Serialize, Deserialize, Clone)]
pub struct HybridBuilder(Vec<Label>);

impl Default for HybridBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl HybridBuilder {
    /// Create an empty hybrid builder
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Add another upstream to the hybrid upstream about to build
    pub fn add_tag(mut self, tag: impl Into<Label>) -> Self {
        self.0.push(tag.into());
        self
    }
}

#[async_trait]
impl AsyncTryInto<Upstream> for HybridBuilder {
    type Error = QHandleError;

    async fn async_try_into(self) -> Result<Upstream> {
        Ok(Upstream::Hybrid(self.0))
    }
}

/// A builder for DNS over HTTPS upstream
#[cfg(any(feature = "doh-rustls", feature = "doh-native-tls"))]
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub struct HttpsBuilder {
    /// The URL of the DoH server. e.g. `https://cloudflare-dns.com/dns-query`
    pub uri: String,
    /// The address of the server. e.g. `1.1.1.1` for Cloudflare DNS.
    pub addr: IpAddr,
    /// The Proxy URL used to connect the upstream server. Supporting HTTP and SOCKS5 proxy formats.
    pub proxy: Option<String>,
    /// Timeout length
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    /// SNI
    #[serde(default)]
    pub sni: bool,
}

#[cfg(any(feature = "doh-rustls", feature = "doh-native-tls"))]
#[async_trait]
impl AsyncTryInto<Upstream> for HttpsBuilder {
    type Error = QHandleError;

    async fn async_try_into(self) -> Result<Upstream> {
        Ok(Upstream::Others(Arc::new(ConnPool::new(
            Https::new(self.uri, self.addr, self.proxy, self.sni).await?,
            Duration::from_secs(self.timeout),
        )?)))
    }
}

/// A builder for UDP upstream
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub struct UdpBuilder {
    /// Address of the remote server
    pub addr: SocketAddr,
    /// Timeout length
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

#[async_trait]
impl AsyncTryInto<Upstream> for UdpBuilder {
    type Error = QHandleError;

    async fn async_try_into(self) -> Result<Upstream> {
        Ok(Upstream::Others(Arc::new(ConnPool::new(
            Udp::new(self.addr).await?,
            Duration::from_secs(self.timeout),
        )?)))
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
/// The builder for `Upstream`
pub enum UpstreamBuilder {
    /// Race various different upstreams concurrently. You can use it recursively, meaning Hybrid over (Hybrid over (DoH + UDP) + UDP) is legal.
    Hybrid(HybridBuilder),
    /// UDP connection.
    Udp(UdpBuilder),
    #[cfg(any(feature = "doh-rustls", feature = "doh-native-tls"))]
    /// HTTPS connection.
    Https(HttpsBuilder),
}

#[async_trait]
impl AsyncTryInto<Upstream> for UpstreamBuilder {
    /// Build the Upstream from an UpstreamBuilder
    async fn async_try_into(self) -> std::result::Result<Upstream, QHandleError> {
        Ok(match self {
            Self::Hybrid(v) => v.async_try_into().await?,

            // UDP Upstream
            Self::Udp(u) => u.async_try_into().await?,

            #[cfg(any(feature = "doh-rustls", feature = "doh-native-tls"))]
            Self::Https(h) => h.async_try_into().await?,
        })
    }

    type Error = QHandleError;
}
