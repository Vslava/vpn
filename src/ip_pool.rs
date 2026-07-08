use std::collections::HashSet;
use std::net::Ipv4Addr;

use crate::error::Error;

pub struct IpPool {
    next_ip: u32,
    last_ip: u32,
    released: HashSet<u32>,
    netmask: u8,
}

impl IpPool {
    pub fn new(subnet_cidr: &str) -> Result<Self, Error> {
        let (ip_str, prefix_str) = subnet_cidr
            .split_once('/')
            .ok_or_else(|| Error::Config(format!("invalid CIDR '{subnet_cidr}'")))?;

        let network: Ipv4Addr = ip_str
            .parse()
            .map_err(|_| Error::Config(format!("invalid subnet IP '{ip_str}'")))?;

        let prefix: u8 = prefix_str
            .parse()
            .map_err(|_| Error::Config(format!("invalid prefix '{prefix_str}'")))?;

        if prefix == 0 || prefix > 30 {
            return Err(Error::Config(format!(
                "unsupported prefix /{prefix} — must be 1-30"
            )));
        }

        let net_u32 = u32::from(network);
        let host_bits = 32 - prefix;
        let host_mask = (1u64 << host_bits) - 1;
        let broadcast = net_u32 + host_mask as u32;

        let first_usable = net_u32 + 1;
        let server_ip_u32 = first_usable;
        let first_client_u32 = first_usable + 1;
        let last_usable = broadcast.saturating_sub(1);

        if first_client_u32 > last_usable {
            return Err(Error::Config(format!(
                "subnet {subnet_cidr} has not enough IPs for clients"
            )));
        }

        let pool_size = last_usable - first_client_u32 + 1;
        tracing::info!(
            subnet = %subnet_cidr,
            server_ip = %Ipv4Addr::from(server_ip_u32.to_be_bytes()),
            pool_size,
            "IP pool initialized"
        );

        Ok(Self {
            next_ip: first_client_u32,
            last_ip: last_usable,
            released: HashSet::new(),
            netmask: prefix,
        })
    }

    pub fn server_ip(subnet_cidr: &str) -> Result<Ipv4Addr, Error> {
        let (ip_str, _) = subnet_cidr
            .split_once('/')
            .ok_or_else(|| Error::Config(format!("invalid CIDR '{subnet_cidr}'")))?;
        let network: Ipv4Addr = ip_str
            .parse()
            .map_err(|_| Error::Config(format!("invalid subnet IP '{ip_str}'")))?;
        let net_u32 = u32::from(network);
        Ok(Ipv4Addr::from((net_u32 + 1).to_be_bytes()))
    }

    pub fn netmask(&self) -> u8 {
        self.netmask
    }

    pub fn allocate(&mut self) -> Option<Ipv4Addr> {
        if let Some(&ip) = self.released.iter().next() {
            self.released.remove(&ip);
            return Some(Ipv4Addr::from(ip.to_be_bytes()));
        }

        if self.next_ip <= self.last_ip {
            let ip = self.next_ip;
            self.next_ip += 1;
            return Some(Ipv4Addr::from(ip.to_be_bytes()));
        }

        None
    }

    pub fn release(&mut self, ip: Ipv4Addr) {
        self.released.insert(u32::from(ip));
    }

    pub fn count_free(&self) -> usize {
        self.released.len() + (self.last_ip.saturating_sub(self.next_ip.saturating_sub(1)) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_allocate_sequence() {
        let mut pool = IpPool::new("10.0.0.0/24").unwrap();
        assert_eq!(pool.allocate(), Some(Ipv4Addr::new(10, 0, 0, 2)));
        assert_eq!(pool.allocate(), Some(Ipv4Addr::new(10, 0, 0, 3)));
        assert_eq!(pool.allocate(), Some(Ipv4Addr::new(10, 0, 0, 4)));
    }

    #[test]
    fn test_pool_release_reuse() {
        let mut pool = IpPool::new("10.0.0.0/24").unwrap();
        let ip1 = pool.allocate().unwrap();
        pool.allocate().unwrap();
        pool.release(ip1);
        assert_eq!(pool.allocate(), Some(ip1));
    }

    #[test]
    fn test_server_ip() {
        assert_eq!(
            IpPool::server_ip("10.0.0.0/24").unwrap(),
            Ipv4Addr::new(10, 0, 0, 1)
        );
        assert_eq!(
            IpPool::server_ip("10.100.0.0/16").unwrap(),
            Ipv4Addr::new(10, 100, 0, 1)
        );
        assert_eq!(
            IpPool::server_ip("192.168.1.0/24").unwrap(),
            Ipv4Addr::new(192, 168, 1, 1)
        );
    }

    #[test]
    fn test_pool_exhaustion() {
        let mut pool = IpPool::new("10.0.0.0/30").unwrap();
        assert!(pool.allocate().is_some());
        assert!(pool.allocate().is_none());
    }

    #[test]
    fn test_pool_too_small() {
        assert!(IpPool::new("10.0.0.0/31").is_err());
        assert!(IpPool::new("10.0.0.0/0").is_err());
        assert!(IpPool::new("10.0.0.0/32").is_err());
    }
}
