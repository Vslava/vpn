use std::net::Ipv4Addr;

use netlink_packet_route::link::LinkAttribute;
use netlink_packet_route::route::{RouteAddress, RouteAttribute, RouteMessage};
use netlink_packet_route::AddressFamily;
use rtnetlink::{new_connection, Error as NetlinkError, Handle, RouteMessageBuilder};
use tokio_stream::StreamExt;

use crate::error::Error;

fn is_eexist(e: &NetlinkError) -> bool {
    matches!(e, NetlinkError::NetlinkError(msg) if msg.raw_code() == -17)
}

fn eexist_ok(result: Result<(), NetlinkError>) -> Result<(), Error> {
    match result {
        Ok(()) => Ok(()),
        Err(ref e) if is_eexist(e) => Ok(()),
        Err(e) => Err(Error::Io(std::io::Error::other(e))),
    }
}

#[derive(Debug, Clone)]
pub struct DefaultRoute {
    pub gateway: Ipv4Addr,
    pub ifindex: u32,
    pub ifname: String,
    pub metric: u32,
}

fn create_handle() -> Result<(Handle, tokio::task::JoinHandle<()>), Error> {
    let (conn, handle, _) = new_connection().map_err(Error::Io)?;
    let task = tokio::spawn(conn);
    Ok((handle, task))
}

async fn get_ifname(handle: &Handle, ifindex: u32) -> Result<String, Error> {
    let mut links = handle.link().get().execute();
    while let Some(msg) = links.next().await {
        let link = msg.map_err(|e| Error::Io(std::io::Error::other(e)))?;
        if link.header.index != ifindex {
            continue;
        }
        for attr in &link.attributes {
            if let LinkAttribute::IfName(name) = attr {
                return Ok(name.clone());
            }
        }
    }
    Err(Error::Config(format!("interface with index {ifindex} not found")))
}

async fn get_ifindex(handle: &Handle, name: &str) -> Result<u32, Error> {
    let mut links = handle.link().get().execute();
    while let Some(msg) = links.next().await {
        let link = msg.map_err(|e| Error::Io(std::io::Error::other(e)))?;
        for attr in &link.attributes {
            if let LinkAttribute::IfName(n) = attr {
                if n == name {
                    return Ok(link.header.index);
                }
            }
        }
    }
    Err(Error::Config(format!("interface '{name}' not found")))
}

fn find_default_route(msg: &RouteMessage) -> Option<(Ipv4Addr, u32, u32)> {
    if msg.header.address_family != AddressFamily::Inet {
        return None;
    }
    if msg.header.destination_prefix_length != 0 {
        return None;
    }
    let mut gateway = None;
    let mut ifindex = None;
    let mut priority = 0;
    for attr in &msg.attributes {
        match attr {
            RouteAttribute::Gateway(RouteAddress::Inet(gw)) => gateway = Some(*gw),
            RouteAttribute::Oif(idx) => ifindex = Some(*idx),
            RouteAttribute::Priority(p) => priority = *p,
            _ => (),
        }
    }
    Some((gateway?, ifindex?, priority))
}

pub async fn save_default_route() -> Result<Option<DefaultRoute>, Error> {
    let (handle, _task) = create_handle()?;

    let msg = RouteMessageBuilder::<Ipv4Addr>::new().build();
    let mut stream = handle.route().get(msg).execute();

    while let Some(result) = stream.next().await {
        let route = result.map_err(|e| Error::Io(std::io::Error::other(e)))?;
        if let Some((gw, idx, prio)) = find_default_route(&route) {
            let ifname = get_ifname(&handle, idx).await?;
            return Ok(Some(DefaultRoute {
                gateway: gw,
                ifindex: idx,
                ifname,
                metric: prio,
            }));
        }
    }
    Ok(None)
}

pub async fn set_tun_route(tun_ifname: &str, tun_gw: Ipv4Addr) -> Result<(), Error> {
    let (handle, _task) = create_handle()?;
    let ifindex = get_ifindex(&handle, tun_ifname).await?;

    let del_msg = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .build();
    let _ = handle.route().del(del_msg).execute().await;

    let add_msg = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .gateway(tun_gw)
        .output_interface(ifindex)
        .build();

    eexist_ok(
        handle
            .route()
            .add(add_msg)
            .execute()
            .await,
    )?;

    Ok(())
}

pub async fn add_exclude_route(
    server_ip: Ipv4Addr,
    orig: &DefaultRoute,
) -> Result<(), Error> {
    let (handle, _task) = create_handle()?;

    let add_msg = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(server_ip, 32)
        .gateway(orig.gateway)
        .output_interface(orig.ifindex)
        .priority(orig.metric)
        .build();

    eexist_ok(
        handle
            .route()
            .add(add_msg)
            .execute()
            .await,
    )?;

    Ok(())
}

pub async fn restore_route(orig: &DefaultRoute) -> Result<(), Error> {
    let (handle, _task) = create_handle()?;

    let del_msg = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .build();
    let _ = handle.route().del(del_msg).execute().await;

    let add_msg = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
        .gateway(orig.gateway)
        .output_interface(orig.ifindex)
        .priority(orig.metric)
        .build();

    eexist_ok(
        handle
            .route()
            .add(add_msg)
            .execute()
            .await,
    )?;

    Ok(())
}
