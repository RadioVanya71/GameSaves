use crate::snapshots;
use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::{Builder, NamedTempFile};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Semaphore, watch};
use tokio::time::{sleep, timeout};

const PROTOCOL: &[u8] = b"game-saves/1";
const MAX_ADDRESS: u64 = 16 * 1024;
const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
const CONNECTION_WAIT: Duration = Duration::from_secs(6);
const TRANSFER_WAIT: Duration = Duration::from_secs(10 * 60);
const ACK: [u8; 1] = [1];
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn deliver(
    sender: Option<&Sender>,
    world: &World,
    source: &Path,
) -> std::result::Result<String, String> {
    let archive = snapshots::publish(source, &world.local).map_err(|error| error.to_string())?;
    if sender.is_some_and(|sender| sender.try_direct(world, &archive)) {
        return Ok("Сохранение передано напрямую.".into());
    }
    snapshots::copy_version(&archive, &world.shared).map_err(|error| {
        format!(
            "Общая папка недоступна. Локальная версия сохранена: {}. {error}",
            archive.display()
        )
    })?;
    Ok("Сохранение записано в общую папку. Облачный диск доставит его после синхронизации.".into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct World {
    pub shared: PathBuf,
    pub local: PathBuf,
}

impl World {
    pub fn new(shared: PathBuf, app_data: &Path) -> Option<Self> {
        let local = app_data.join("versions").join(shared.file_name()?);
        Some(Self { shared, local })
    }

    fn name(&self) -> &str {
        self.shared
            .file_name()
            .and_then(|name| name.to_str())
            .expect("validated world code")
    }

    fn addresses(&self) -> PathBuf {
        self.shared.join(".peers")
    }

    fn address_file(&self, id: EndpointId) -> PathBuf {
        self.addresses().join(format!("{id}.json"))
    }
}

pub struct Network {
    runtime: tokio::runtime::Runtime,
    worlds: watch::Sender<Vec<World>>,
    endpoint: watch::Receiver<Option<Endpoint>>,
}

impl Network {
    pub fn start(app_data: PathBuf) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let (worlds, configuration) = watch::channel(Vec::new());
        let (ready, endpoint) = watch::channel(None);
        runtime.spawn(run_network(app_data, configuration, ready));
        Ok(Self {
            runtime,
            worlds,
            endpoint,
        })
    }

    pub fn configure(&self, worlds: Vec<World>) {
        self.worlds.send_if_modified(|previous| {
            if *previous == worlds {
                return false;
            }
            *previous = worlds;
            true
        });
    }

    pub fn sender(&self) -> Sender {
        Sender {
            handle: self.runtime.handle().clone(),
            endpoint: self.endpoint.clone(),
        }
    }
}

pub struct Sender {
    handle: tokio::runtime::Handle,
    endpoint: watch::Receiver<Option<Endpoint>>,
}

impl Sender {
    pub fn try_direct(&self, world: &World, version: &Path) -> bool {
        let Some(endpoint) = self.endpoint.borrow().clone() else {
            return false;
        };
        let _ = advertise(world, &endpoint.addr());
        let peers = peer_addresses(world, endpoint.id());
        if peers.is_empty() {
            return false;
        }
        self.handle.block_on(async {
            let mut transfers = tokio::task::JoinSet::new();
            for addr in peers {
                let endpoint = endpoint.clone();
                let world = world.name().to_owned();
                let version = version.to_path_buf();
                transfers
                    .spawn(async move { send_version(&endpoint, addr, &world, &version).await });
            }
            let mut delivered = true;
            while let Some(result) = transfers.join_next().await {
                delivered &= matches!(result, Ok(Ok(())));
            }
            delivered
        })
    }
}

fn endpoint_key(app_data: &Path) -> Result<SecretKey> {
    let path = app_data.join("pc-key");
    let read_key = || -> Result<SecretKey> {
        let bytes = fs::read(&path)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| "invalid PC key")?;
        Ok(SecretKey::from_bytes(&bytes))
    };
    if path.exists() {
        return read_key();
    }
    let key = SecretKey::generate();
    fs::create_dir_all(app_data)?;
    let mut pending = NamedTempFile::new_in(app_data)?;
    pending.write_all(&key.to_bytes())?;
    pending.as_file().sync_all()?;
    match pending.persist_noclobber(path.clone()) {
        Ok(_) => Ok(key),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => read_key(),
        Err(error) => Err(error.into()),
    }
}

async fn bind_endpoint(key: SecretKey, relay: RelayMode) -> Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .relay_mode(relay)
        .secret_key(key)
        .alpns(vec![PROTOCOL.to_vec()])
        .bind()
        .await?)
}

async fn run_network(
    app_data: PathBuf,
    mut worlds: watch::Receiver<Vec<World>>,
    ready: watch::Sender<Option<Endpoint>>,
) {
    if worlds.wait_for(|worlds| !worlds.is_empty()).await.is_err() {
        return;
    }
    let Ok(key) = endpoint_key(&app_data) else {
        return;
    };
    let Ok(endpoint) = bind_endpoint(key, RelayMode::Default).await else {
        return;
    };
    let _ = ready.send(Some(endpoint.clone()));
    let updater = endpoint.clone();
    let mut configuration = worlds.clone();
    tokio::spawn(async move {
        loop {
            let worlds = configuration.borrow().clone();
            let address = updater.addr();
            let _ = tokio::task::spawn_blocking(move || {
                for world in &worlds {
                    let _ = advertise(world, &address);
                }
            })
            .await;
            tokio::select! {
                changed = configuration.changed() => {
                    if changed.is_err() { break; }
                }
                _ = sleep(Duration::from_secs(3)) => {}
            }
        }
    });
    let slots = Arc::new(Semaphore::new(8));
    while let Some(incoming) = endpoint.accept().await {
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            incoming.refuse();
            continue;
        };
        let worlds = worlds.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let Ok(Ok(connection)) = timeout(CONNECTION_WAIT, incoming).await else {
                return;
            };
            let worlds = worlds.borrow().clone();
            let _ = timeout(TRANSFER_WAIT, receive_version(connection, &worlds)).await;
        });
    }
}

fn read_address(path: &Path) -> Result<EndpointAddr> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_ADDRESS + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ADDRESS {
        return Err("peer address too large".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn advertise(world: &World, address: &EndpointAddr) -> Result<()> {
    let path = world.address_file(address.id);
    if read_address(&path).is_ok_and(|previous| previous == *address) {
        return Ok(());
    }
    let directory = world.addresses();
    fs::create_dir_all(&directory)?;
    let mut pending = NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut pending, address)?;
    pending.persist(path)?;
    Ok(())
}

fn peer_addresses(world: &World, ourselves: EndpointId) -> Vec<EndpointAddr> {
    let Ok(entries) = fs::read_dir(world.addresses()) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "json" || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let id: EndpointId = path.file_stem()?.to_str()?.parse().ok()?;
            if id == ourselves || !seen.insert(id) {
                return None;
            }
            let address = read_address(&path).ok()?;
            (id == address.id).then_some(address)
        })
        .collect()
}

fn direct(connection: &Connection) -> bool {
    connection
        .paths()
        .iter()
        .any(|path| path.is_selected() && path.is_ip())
}

async fn direct_connection(endpoint: &Endpoint, addr: EndpointAddr) -> Result<Connection> {
    let connection = endpoint.connect(addr, PROTOCOL).await?;
    while !direct(&connection) {
        sleep(Duration::from_millis(80)).await;
    }
    Ok(connection)
}

async fn send_version(
    endpoint: &Endpoint,
    addr: EndpointAddr,
    world: &str,
    version: &Path,
) -> Result<()> {
    let mut archive = tokio::fs::File::open(version).await?;
    let size = archive.metadata().await?.len();
    if size == 0 || size > MAX_ARCHIVE {
        return Err("archive size outside direct transfer limit".into());
    }
    let name = version
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid version name")?;
    let connection = timeout(CONNECTION_WAIT, direct_connection(endpoint, addr)).await??;
    let result = timeout(TRANSFER_WAIT, async {
        let (mut send, mut receive) = connection.open_bi().await?;
        send.write_all(&[world.len() as u8, name.len() as u8])
            .await?;
        send.write_all(world.as_bytes()).await?;
        send.write_all(name.as_bytes()).await?;
        send.write_all(&size.to_be_bytes()).await?;
        let mut buffer = vec![0; 64 * 1024];
        loop {
            if !direct(&connection) {
                return Err("direct route disappeared".into());
            }
            let read = archive.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            send.write_all(&buffer[..read]).await?;
        }
        send.finish()?;
        let mut ack = [0];
        receive.read_exact(&mut ack).await?;
        if ack != ACK {
            return Err("missing delivery confirmation".into());
        }
        Ok(())
    })
    .await;
    connection.close(0u8.into(), b"done");
    result?
}

async fn receive_version(connection: Connection, worlds: &[World]) -> Result<()> {
    let (mut send, mut receive) = timeout(CONNECTION_WAIT, connection.accept_bi()).await??;
    let mut lengths = [0; 2];
    receive.read_exact(&mut lengths).await?;
    let mut world = vec![0; lengths[0].into()];
    let mut name = vec![0; lengths[1].into()];
    receive.read_exact(&mut world).await?;
    receive.read_exact(&mut name).await?;
    let name = String::from_utf8(name)?;
    if !snapshots::valid_version_name(&name) {
        return Err("invalid version name".into());
    }
    let world = worlds
        .iter()
        .find(|candidate| candidate.name().as_bytes() == world)
        .ok_or("unknown world")?;
    let address = read_address(&world.address_file(connection.remote_id()))?;
    if address.id != connection.remote_id() {
        return Err("unauthorized peer".into());
    }
    let mut size = [0; 8];
    receive.read_exact(&mut size).await?;
    let size = u64::from_be_bytes(size);
    if size == 0 || size > MAX_ARCHIVE {
        return Err("invalid archive size".into());
    }
    tokio::fs::create_dir_all(&world.local).await?;
    let pending = Builder::new()
        .suffix(".pending")
        .tempfile_in(&world.local)?;
    let (file, path) = pending.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let received = tokio::io::copy(&mut (&mut receive).take(size + 1), &mut file).await?;
    if received != size {
        return Err("incomplete or oversized archive".into());
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    let destination = world.local.join(name);
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut archive = zip::ZipArchive::new(File::open(&path)?)?;
        let mut save = archive.by_name("save")?;
        if save.size() == 0
            || save.size() > MAX_ARCHIVE
            || save.compression() != zip::CompressionMethod::Stored
        {
            return Err("invalid save entry".into());
        }
        io::copy(&mut save, &mut io::sink())?;
        drop(save);
        drop(archive);
        match path.persist_noclobber(destination) {
            Ok(()) => Ok(()),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.into()),
        }
    })
    .await??;
    send.write_all(&ACK).await?;
    send.finish()?;
    let _ = send.stopped().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn direct_delivery_and_offline_fallback() -> Result<()> {
        let root = tempfile::tempdir()?;
        let shared = snapshots::world_directory(root.path(), "subnautica-2")
            .map_err(|error| error.to_string())?;
        let from = World::new(shared.clone(), &root.path().join("sender")).unwrap();
        let to = World::new(shared.clone(), &root.path().join("recipient")).unwrap();
        let recipient = bind_endpoint(SecretKey::generate(), RelayMode::Disabled).await?;
        let bound_v4 = recipient
            .bound_sockets()
            .into_iter()
            .find(SocketAddr::is_ipv4)
            .ok_or("missing IPv4 socket")?;
        let target = SocketAddr::from((Ipv4Addr::LOCALHOST, bound_v4.port()));
        advertise(&to, &EndpointAddr::new(recipient.id()).with_ip_addr(target))?;
        let outgoing = bind_endpoint(SecretKey::generate(), RelayMode::Disabled).await?;
        let (_ready, endpoint) = watch::channel(Some(outgoing.clone()));
        let receiver = recipient.clone();
        let destination = to.clone();
        let accept = tokio::spawn(async move {
            let connection = receiver.accept().await.ok_or("endpoint closed")?.await?;
            receive_version(connection, &[destination]).await
        });
        let source = root.path().join("savegame_0.sav");
        fs::write(&source, b"world version from peer")?;
        let sender = Sender {
            handle: tokio::runtime::Handle::current(),
            endpoint: endpoint.clone(),
        };
        let (copy_from, copy_source) = (from.clone(), source.clone());
        let status =
            tokio::task::spawn_blocking(move || deliver(Some(&sender), &copy_from, &copy_source))
                .await??;
        assert_eq!(status, "Сохранение передано напрямую.");
        timeout(CONNECTION_WAIT, accept).await???;
        assert_eq!(snapshots::versions(&to.local)?.len(), 1);
        assert!(snapshots::versions(&shared)?.is_empty());
        recipient.close().await;

        fs::write(&source, b"world version for a friend who is offline")?;
        let sender = Sender {
            handle: tokio::runtime::Handle::current(),
            endpoint,
        };
        let status =
            tokio::task::spawn_blocking(move || deliver(Some(&sender), &from, &source)).await??;
        assert!(status.contains("записано в общую папку"));
        assert_eq!(snapshots::versions(&shared)?.len(), 1);
        outgoing.close().await;
        Ok(())
    }
}
