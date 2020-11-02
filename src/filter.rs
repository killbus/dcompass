use crate::parser::{Parsed, Rule, Upstream, UpstreamKind};
use anyhow::{anyhow, Result};
use dmatcher::Dmatcher;
use hashbrown::HashMap;
use log::*;
use std::{net::SocketAddr, time::Duration};
use tokio::{fs::File, prelude::*};
use tokio_compat_02::FutureExt;
use trust_dns_proto::{
    op::{response_code::ResponseCode, Message},
    rr::{Record, RecordType},
    xfer::dns_request::DnsRequestOptions,
};
use trust_dns_resolver::{config::*, TokioAsyncResolver};

pub struct Filter {
    resolvers: HashMap<u32, TokioAsyncResolver>,
    default_tag: u32,
    disable_ipv6: bool,
    matcher: Dmatcher<u32>,
    dsts: Vec<u32>,
}

impl Filter {
    async fn insert_rules(rules: Vec<Rule>) -> Result<(Dmatcher<u32>, Vec<u32>)> {
        let mut matcher = Dmatcher::new();
        let mut v = vec![];
        for r in rules {
            println!("here!");
            let mut file = File::open(r.path).await?;
            let mut data = String::new();

            file.read_to_string(&mut data).await?;
            matcher.insert_lines(data, r.dst)?;
            v.push(r.dst);
        }
        Ok((matcher, v))
    }

    async fn insert_upstreams(
        upstreams: Vec<Upstream>,
    ) -> Result<HashMap<u32, TokioAsyncResolver>> {
        let mut r = HashMap::new();

        for upstream in upstreams {
            let mut opts = ResolverOpts::default();
            opts.cache_size = upstream.cache_size;
            opts.distrust_nx_responses = false; // This slows down resolution and does no good.
            opts.timeout = Duration::from_secs(upstream.timeout);

            r.insert(
                upstream.tag,
                TokioAsyncResolver::tokio(
                    ResolverConfig::from_parts(
                        None,
                        vec![],
                        match upstream.method {
                            UpstreamKind::Tls(tls_name) => NameServerConfigGroup::from_ips_tls(
                                &upstream.ips,
                                upstream.port,
                                tls_name,
                            ),
                            UpstreamKind::Udp => {
                                NameServerConfigGroup::from_ips_clear(&upstream.ips, upstream.port)
                            }
                            UpstreamKind::Https(tls_name) => NameServerConfigGroup::from_ips_tls(
                                &upstream.ips,
                                upstream.port,
                                tls_name,
                            ),
                        },
                    ),
                    opts,
                )
                .compat()
                .await?,
            );
        }
        Ok(r)
    }

    pub async fn new(data: &str) -> Result<(Self, SocketAddr, u32, LevelFilter)> {
        let p: Parsed = serde_json::from_str(data)?;
        let (matcher, dsts) = Filter::insert_rules(p.rules).await?;
        let filter = Filter {
            matcher,
            resolvers: Filter::insert_upstreams(p.upstreams).await?,
            default_tag: p.default_tag,
            disable_ipv6: p.disable_ipv6,
            dsts,
        };
        filter.check(filter.default_tag)?;
        Ok((filter, p.address, p.workers, p.verbosity))
    }

    pub fn check(&self, default: u32) -> Result<()> {
        for dst in &self.dsts {
            self.resolvers
                .get(&dst)
                .ok_or_else(|| anyhow!("Missing resolver: {}", dst))?;
        }
        self.resolvers
            .get(&default)
            .ok_or_else(|| anyhow!("Missing default resolver: {}", default))?;
        Ok(())
    }

    fn get_resolver(&self, domain: &str) -> Result<&TokioAsyncResolver> {
        Ok(match self.matcher.matches(domain)? {
            Some(u) => {
                info!("Routed via {}", u);
                self.resolvers
                    .get(&u)
                    .ok_or_else(|| anyhow!("Missing resolver: {}", &u))?
                // These won't be reached unless it is unchecked.
            }
            None => {
                info!("Routed via default: {}", &self.default_tag);
                self.resolvers
                    .get(&self.default_tag)
                    .ok_or_else(|| anyhow!("Missing default resolver: {}", &self.default_tag))?
            }
        })
    }

    pub async fn resolve(
        &self,
        domain: String,
        qtype: RecordType,
        mut req: Message,
    ) -> Result<Message> {
        Ok(if (qtype == RecordType::AAAA) && (self.disable_ipv6) {
            // If `disable_ipv6` has been set, return immediately NXDomain.
            Message::error_msg(req.id(), req.op_code(), ResponseCode::NXDomain)
        } else {
            // Get the corresponding resolver
            match self
                .get_resolver(domain.as_str())?
                .lookup(
                    domain,
                    qtype,
                    DnsRequestOptions {
                        expects_multiple_responses: false,
                    },
                )
                .compat()
                .await
            {
                Err(e) => {
                    warn!("Resolve failed: {}", e);
                    // TODO: We should specify different errors and return them back respectively.
                    Message::error_msg(req.id(), req.op_code(), ResponseCode::NXDomain)
                }
                Ok(r) => {
                    req.add_answers(r.record_iter().cloned().collect::<Vec<Record>>());
                    req
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Filter;
    use tokio_test::block_on;

    #[test]
    fn parse() {
        assert_eq!(
            block_on(Filter::new(include_str!("../configs/default.json"))).is_ok(),
            true
        );
    }

    #[test]
    fn check_fail_rule() {
        // Notice that data dir is relative to cargo test path.
        assert_eq!(
            block_on(Filter::new(include_str!("../configs/fail_rule.json"))).is_err(),
            true
        );
    }

    #[test]
    fn check_fail_default() {
        assert_eq!(
            block_on(Filter::new(include_str!("../configs/fail_default.json"))).is_err(),
            true
        );
    }
}
