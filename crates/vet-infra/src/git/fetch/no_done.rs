//! Connecting for a fetch without ever asking a protocol V1 server for `no-done`.
//!
//! gix 0.88 reads a V1 reply to a round of haves only up to its `NAK`, even after the server said
//! `ready`. With `no-done` the server follows that `NAK` with the pack in the same reply; over HTTP
//! nobody reads it, and the next round waits forever for gix's HTTP thread, which is still handing
//! over the pack. Without `no-done` a server sends no pack before `done`. Azure DevOps speaks V1 only.

use std::any::Any;
use std::borrow::Cow;

use gix::bstr::BStr;
use gix::protocol::transport::client::blocking_io::{RequestWriter, SetServiceResponse, Transport, connect};
use gix::protocol::transport::client::{Capabilities, Error, MessageKind, TransportWithoutIO, WriteMode};
use gix::protocol::transport::{Protocol, Service};
use gix::remote::{Connection, Direction};
use gix::sec::identity::Account;

/// Connects to the fetch URL of `remote` as `gix` does, through a transport that hides `no-done`.
pub(super) fn connect<'remote, 'repo>(
    remote: &'remote gix::Remote<'repo>,
) -> gix::Result<Connection<'remote, 'static, 'repo, Box<dyn Transport + Send>>> {
    let (url, version) = remote.sanitized_url_and_version(Direction::Fetch)?;
    let ssh = match url.scheme {
        gix::url::Scheme::Ssh => remote.repo().ssh_connect_options()?,
        _ => Default::default(),
    };
    let trace = remote
        .repo()
        .config_snapshot()
        .boolean(gix::config::tree::Gitoxide::TRACE_PACKET)
        .unwrap_or(false);
    let transport = connect::connect(url, connect::Options { version, ssh, trace })?;
    Ok(remote.to_connection_with_transport(Box::new(WithoutNoDone(transport)) as Box<dyn Transport + Send>))
}

struct WithoutNoDone<T>(T);

impl<T: TransportWithoutIO> TransportWithoutIO for WithoutNoDone<T> {
    fn set_identity(&mut self, identity: Account) -> Result<(), Error> {
        self.0.set_identity(identity)
    }

    fn to_url(&self) -> Cow<'_, BStr> {
        self.0.to_url()
    }

    fn supported_protocol_versions(&self) -> &[Protocol] {
        self.0.supported_protocol_versions()
    }

    fn connection_persists_across_multiple_requests(&self) -> bool {
        self.0.connection_persists_across_multiple_requests()
    }

    fn configure(&mut self, config: &dyn Any) -> gix::ExnResult {
        self.0.configure(config)
    }
}

impl<T: Transport> Transport for WithoutNoDone<T> {
    fn handshake<'a>(
        &mut self,
        service: Service,
        extra_parameters: &'a [(&'a str, Option<&'a str>)],
    ) -> Result<SetServiceResponse<'_>, Error> {
        let mut response = self.0.handshake(service, extra_parameters)?;
        if response.actual_protocol != Protocol::V2 {
            response.capabilities = without_no_done(&response.capabilities);
        }
        Ok(response)
    }

    fn request(
        &mut self,
        write_mode: WriteMode,
        on_into_read: MessageKind,
        trace: bool,
    ) -> Result<RequestWriter<'_>, Error> {
        self.0.request(write_mode, on_into_read, trace)
    }
}

/// `capabilities` without `no-done`, spelled as V1 lists them behind the first ref. Values such as
/// `symref=HEAD:refs/heads/main` are kept: the refs are read with them.
fn without_no_done(capabilities: &Capabilities) -> Capabilities {
    let mut line = vec![0u8];
    for capability in capabilities.iter().filter(|c| c.name() != "no-done") {
        if line.len() > 1 {
            line.push(b' ');
        }
        line.extend_from_slice(capability.name());
        if let Some(value) = capability.value() {
            line.push(b'=');
            line.extend_from_slice(value);
        }
    }
    Capabilities::from_bytes(&line).map_or_else(|_| capabilities.clone(), |(kept, _)| kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_v1_advertisement_loses_no_done_and_keeps_the_rest_with_their_values() {
        let (advertised, _) = Capabilities::from_bytes(
            b"\0multi_ack thin-pack side-band-64k multi_ack_detailed no-done shallow symref=HEAD:refs/heads/main",
        )
        .unwrap();

        let kept = without_no_done(&advertised);

        let names: Vec<String> = kept.iter().map(|c| c.name().to_string()).collect();
        assert_eq!(
            names,
            [
                "multi_ack",
                "thin-pack",
                "side-band-64k",
                "multi_ack_detailed",
                "shallow",
                "symref"
            ]
        );
        let symref = kept.capability("symref").and_then(|c| c.value()).map(|v| v.to_string());
        assert_eq!(symref.as_deref(), Some("HEAD:refs/heads/main"));
    }
}
