//! Startup-only WSL2 NAT discovery. No subprocesses, subscriptions or route mutation.
use super::ServerSettings;
use futures_util::TryStreamExt;
use rtnetlink::packet_route::{
    address::AddressAttribute,
    link::LinkFlags,
    route::{RouteAddress, RouteAttribute, RouteMessage, RouteType},
    AddressFamily,
};
use std::net::{IpAddr, Ipv4Addr};

const SCAN_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub interface: String,
    pub ifindex: u32,
    pub gateway: Ipv4Addr,
    pub server_url: String,
}

// Abort the socket driver even when timeout/cancellation drops discovery midway.
struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn netlink_error(_: impl std::fmt::Display) -> String {
    "netlink_failed: could not read WSL2 NAT network state".into()
}

fn scan_bound(count: usize) -> Result<(), String> {
    if count > SCAN_LIMIT {
        Err("netlink_scan_limit: network inventory exceeds discovery limit".into())
    } else {
        Ok(())
    }
}

async fn link(handle: &rtnetlink::Handle, name: &str) -> Result<u32, String> {
    let mut links = handle.link().get().match_name(name).execute();
    let item = links
        .try_next()
        .await
        .map_err(|error| match error {
            rtnetlink::Error::NetlinkError(ref message) if message.raw_code() == -libc::ENODEV => {
                "interface_missing: WSL2 NAT interface is unavailable".into()
            }
            _ => netlink_error(error),
        })?
        .ok_or("interface_missing: WSL2 NAT interface is unavailable")?;
    if links.try_next().await.map_err(netlink_error)?.is_some() {
        return Err("interface_ambiguous: interface name is not unique".into());
    }
    if !item.header.flags.contains(LinkFlags::Up) {
        return Err("interface_not_ready: WSL2 NAT interface is down".into());
    }
    Ok(item.header.index)
}

async fn addresses(handle: &rtnetlink::Handle, index: u32) -> Result<Vec<Ipv4Addr>, String> {
    let mut stream = handle
        .address()
        .get()
        .set_link_index_filter(index)
        .execute();
    let mut result = Vec::new();
    let mut count = 0;
    while let Some(address) = stream.try_next().await.map_err(netlink_error)? {
        count += 1;
        scan_bound(count)?;
        if address.header.family == AddressFamily::Inet {
            for attr in address.attributes {
                if let AddressAttribute::Address(IpAddr::V4(ip))
                | AddressAttribute::Local(IpAddr::V4(ip)) = attr
                {
                    result.push(ip);
                }
            }
        }
    }
    result.sort();
    result.dedup();
    if result.is_empty() {
        return Err("interface_not_ready: WSL2 NAT interface has no IPv4 address".into());
    }
    Ok(result)
}

fn route_candidate(route: &RouteMessage, index: u32) -> Option<(u32, Ipv4Addr)> {
    if route.header.address_family != AddressFamily::Inet
        || route.header.destination_prefix_length != 0
        || route.header.source_prefix_length != 0
        || route.header.tos != 0
        || route.header.kind != RouteType::Unicast
    {
        return None;
    }
    let mut table = u32::from(route.header.table);
    let mut oif = None;
    let mut gateway = None;
    let mut metric = 0;
    for attr in &route.attributes {
        match attr {
            RouteAttribute::Table(value) => table = *value,
            RouteAttribute::Oif(value) => oif = Some(*value),
            RouteAttribute::Gateway(RouteAddress::Inet(ip)) => gateway = Some(*ip),
            RouteAttribute::Priority(value) => metric = *value,
            RouteAttribute::MultiPath(_)
            | RouteAttribute::Via(_)
            | RouteAttribute::NhId(_)
            | RouteAttribute::Iif(_)
            | RouteAttribute::Mark(_) => return None,
            _ => {}
        }
    }
    (table == 254 && oif == Some(index)).then_some((metric, gateway?))
}

fn select_gateway(candidates: &[(u32, Ipv4Addr)], own: &[Ipv4Addr]) -> Result<Ipv4Addr, String> {
    let best = candidates
        .iter()
        .map(|(metric, _)| *metric)
        .min()
        .ok_or("gateway_missing: no supported IPv4 main-table default gateway")?;
    let mut matches = candidates.iter().filter(|(metric, _)| *metric == best);
    let gateway = matches.next().expect("minimum has a candidate").1;
    if matches.next().is_some() {
        return Err("gateway_ambiguous: multiple equally preferred default gateways".into());
    }
    if gateway.is_unspecified()
        || gateway.is_loopback()
        || gateway.is_multicast()
        || gateway.is_broadcast()
        || own.contains(&gateway)
    {
        return Err("gateway_invalid: default gateway is not a usable host address".into());
    }
    Ok(gateway)
}

async fn gateway(
    handle: &rtnetlink::Handle,
    index: u32,
    own: &[Ipv4Addr],
) -> Result<Ipv4Addr, String> {
    let query = rtnetlink::RouteMessageBuilder::<Ipv4Addr>::new().build();
    let mut routes = handle.route().get(query).execute();
    let mut candidates = Vec::new();
    let mut count = 0;
    while let Some(route) = routes.try_next().await.map_err(netlink_error)? {
        count += 1;
        scan_bound(count)?;
        if let Some(candidate) = route_candidate(&route, index) {
            candidates.push(candidate);
        }
    }
    select_gateway(&candidates, own)
}

pub(super) async fn resolve(settings: &ServerSettings) -> Result<Target, String> {
    let (connection, handle, _) = rtnetlink::new_connection().map_err(netlink_error)?;
    let _driver = ConnectionTask(tokio::spawn(connection));
    let interface = settings.wsl2_interface();
    let index = link(&handle, interface).await?;
    let own = addresses(&handle, index).await?;
    let ip = gateway(&handle, index, &own).await?;
    // Fail closed on an observed topology change during this bounded read.
    if link(&handle, interface).await? != index
        || addresses(&handle, index).await? != own
        || gateway(&handle, index, &own).await? != ip
    {
        return Err("network_changed: retry WSL2 NAT startup discovery".into());
    }
    Ok(Target {
        interface: interface.into(),
        ifindex: index,
        gateway: ip,
        server_url: format!(
            "{}://{}:{}",
            settings.wsl2_scheme(),
            ip,
            settings.port.ok_or("server.port is required")?
        ),
    })
}

#[cfg(test)]
#[path = "tests/controller_wsl2_nat_routes.rs"]
mod tests;
