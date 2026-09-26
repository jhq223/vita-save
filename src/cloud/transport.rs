//! Bound individual socket waits, without imposing a total time limit on large saves.
use ureq::{
    Agent, Error,
    config::Config,
    unversioned::{
        resolver::DefaultResolver,
        transport::{
            Buffers, ConnectionDetails, Connector, NextTimeout, RustlsConnector, TcpConnector,
            Transport, time::Duration,
        },
    },
};

pub fn agent(config: Config) -> Agent {
    let connector =
        ().chain(TcpConnector::default())
            .chain(IdleLimit)
            .chain(RustlsConnector::default());
    Agent::with_parts(config, connector, DefaultResolver::default())
}
#[derive(Debug)]
struct IdleLimit;
impl<T: Transport> Connector<T> for IdleLimit {
    type Out = IdleTransport<T>;
    fn connect(
        &self,
        _: &ConnectionDetails<'_>,
        chained: Option<T>,
    ) -> Result<Option<Self::Out>, Error> {
        Ok(chained.map(IdleTransport))
    }
}
#[derive(Debug)]
struct IdleTransport<T>(T);
fn capped(mut timeout: NextTimeout) -> NextTimeout {
    timeout.after = timeout.after.min(Duration::from_secs(10));
    timeout
}
impl<T: Transport> Transport for IdleTransport<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        self.0.transmit_output(amount, capped(timeout))
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        self.0.await_input(capped(timeout))
    }
    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }
    fn is_tls(&self) -> bool {
        self.0.is_tls()
    }
}
