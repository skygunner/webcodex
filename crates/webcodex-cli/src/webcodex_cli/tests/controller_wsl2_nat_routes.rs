use super::*;
use rtnetlink::packet_route::route::RouteHeader;

fn route(ip: Ipv4Addr, metric: u32) -> RouteMessage {
    let mut route = RouteMessage::default();
    route.header.address_family = AddressFamily::Inet;
    route.header.table = RouteHeader::RT_TABLE_MAIN;
    route.header.kind = RouteType::Unicast;
    route.attributes = vec![
        RouteAttribute::Oif(7),
        RouteAttribute::Gateway(RouteAddress::Inet(ip)),
        RouteAttribute::Priority(metric),
    ];
    route
}

#[test]
fn gateway_uses_next_hop_and_unique_lowest_metric() {
    let own = [Ipv4Addr::new(172, 29, 35, 139)];
    let gateway = Ipv4Addr::new(172, 29, 32, 1);
    let routes = [route(Ipv4Addr::new(172, 29, 32, 2), 20), route(gateway, 10)];
    let candidates: Vec<_> = routes
        .iter()
        .filter_map(|r| route_candidate(r, 7))
        .collect();
    assert_eq!(select_gateway(&candidates, &own).unwrap(), gateway);
    assert!(select_gateway(&[(10, gateway), (10, gateway)], &own)
        .unwrap_err()
        .starts_with("gateway_ambiguous"));
    assert!(select_gateway(&[], &own)
        .unwrap_err()
        .starts_with("gateway_missing"));
}

#[test]
fn unsupported_routes_cannot_become_host_candidates() {
    let gateway = Ipv4Addr::new(172, 29, 32, 1);
    let base = route(gateway, 0);
    assert!(route_candidate(&base, 8).is_none());
    for attr in [
        RouteAttribute::Table(100),
        RouteAttribute::MultiPath(vec![]),
        RouteAttribute::NhId(1),
        RouteAttribute::Iif(1),
    ] {
        let mut r = base.clone();
        r.attributes.push(attr);
        assert!(route_candidate(&r, 7).is_none());
    }
    let mut r = base.clone();
    r.header.address_family = AddressFamily::Inet6;
    assert!(route_candidate(&r, 7).is_none());
    let mut r = base.clone();
    r.header.destination_prefix_length = 24;
    assert!(route_candidate(&r, 7).is_none());
    let mut r = base.clone();
    r.header.source_prefix_length = 8;
    assert!(route_candidate(&r, 7).is_none());
    let mut r = base;
    r.attributes
        .retain(|a| !matches!(a, RouteAttribute::Gateway(_)));
    assert!(route_candidate(&r, 7).is_none());
}

#[test]
fn invalid_gateways_and_unbounded_inventory_fail_closed() {
    let own = Ipv4Addr::new(172, 29, 35, 139);
    for ip in [
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::BROADCAST,
        Ipv4Addr::new(224, 1, 2, 3),
        own,
    ] {
        assert!(select_gateway(&[(0, ip)], &[own])
            .unwrap_err()
            .starts_with("gateway_invalid"));
    }
    assert!(scan_bound(SCAN_LIMIT).is_ok());
    assert!(scan_bound(SCAN_LIMIT + 1).is_err());
}
