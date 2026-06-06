use std::net::Ipv4Addr;

#[tokio::test]
async fn test_save_default_route() {
    let route = traffic_sentinel::route::save_default_route()
        .await
        .expect("save_default_route failed");
    eprintln!("Default route: {:?}", route);
    assert!(route.is_some(), "expected a default route to exist");
    let r = route.unwrap();
    assert_ne!(r.gateway, Ipv4Addr::UNSPECIFIED, "gateway should not be 0.0.0.0");
    assert!(r.ifindex > 0, "ifindex should be > 0");
    assert!(!r.ifname.is_empty(), "ifname should not be empty");
    eprintln!("Gateway: {}, Iface: {} (idx {}), metric: {}", r.gateway, r.ifname, r.ifindex, r.metric);
}

#[tokio::test]
async fn test_tun_route_roundtrip() {
    let orig = traffic_sentinel::route::save_default_route()
        .await
        .expect("save failed")
        .expect("no default route");

    let tun_name = "ts0";
    let tun_ip = Ipv4Addr::new(10, 0, 0, 2);

    let tun = traffic_sentinel::tun::create_tun(tun_name, 1400, tun_ip, 30)
        .await
        .expect("create TUN failed");

    traffic_sentinel::route::set_tun_route(tun_name, Ipv4Addr::new(10, 0, 0, 1))
        .await
        .expect("set_tun_route failed");

    traffic_sentinel::route::add_exclude_route(Ipv4Addr::new(8, 8, 8, 8), &orig)
        .await
        .expect("add_exclude_route failed");

    traffic_sentinel::route::restore_route(&orig)
        .await
        .expect("restore_route failed");

    drop(tun);
}
