use std::io;
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tun::AbstractDevice;

pub struct TunDevice {
    device: tun::AsyncDevice,
}

impl TunDevice {
    fn new(device: tun::AsyncDevice) -> Self {
        TunDevice { device }
    }

    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.device.recv(buf).await
    }

    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.device.send(buf).await
    }

    pub fn mtu(&self) -> u16 {
        self.device.mtu().unwrap_or(1500)
    }
}

impl AsyncRead for TunDevice {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.device).poll_read(cx, buf)
    }
}

impl AsyncWrite for TunDevice {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.device).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.device).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.device).poll_shutdown(cx)
    }
}

pub async fn create_tun(
    name: &str,
    mtu: u16,
    ip: Ipv4Addr,
    netmask_bits: u8,
) -> Result<TunDevice, crate::error::Error> {
    let netmask = crate::config::netmask_from_prefix(netmask_bits);
    let mut config = tun::configure();
    config
        .tun_name(name)
        .address(ip)
        .netmask(netmask)
        .mtu(mtu)
        .up();

    match tun::create_as_async(&config) {
        Ok(device) => Ok(TunDevice::new(device)),
        Err(e) => Err(crate::error::Error::Tun(format!("failed to create TUN: {e}"))),
    }
}
