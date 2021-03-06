#[macro_use]
extern crate log;

#[macro_use]
extern crate failure;

use std::net::{IpAddr, SocketAddr};

extern crate settings;

extern crate ipgen;
extern crate rand;
use rand::{thread_rng, Rng};

use std::str;

use failure::Error;

extern crate reqwest;

extern crate althea_kernel_interface;
use althea_kernel_interface::KernelInterface;
use std::sync::Arc;
use std::sync::RwLock;
use std::thread;
use settings::ExitClientDetails;
use althea_types::LocalIdentity;
use std::time::Duration;

extern crate althea_types;

#[derive(Debug, Fail)]
pub enum CluError {
    #[fail(display = "Runtime Error: {:?}", _0)]
    RuntimeError(String),
}

fn linux_generate_wg_keys(SETTINGS: Arc<RwLock<settings::RitaSettings>>) -> Result<(), Error> {
    let mut ki = KernelInterface {};
    let keys = ki.create_wg_keypair()?;
    let wg_public_key = &keys[0];
    let wg_private_key = &keys[1];

    //Mutates settings, intentional side effect
    SETTINGS.write().unwrap().network.wg_private_key = wg_private_key.to_string();
    SETTINGS.write().unwrap().network.wg_public_key = wg_public_key.to_string();

    Ok(())
}

fn openwrt_generate_and_set_wg_keys(
    SETTINGS: Arc<RwLock<settings::RitaSettings>>,
) -> Result<(), Error> {
    let mut ki = KernelInterface {};
    let keys = ki.create_wg_keypair()?;
    let wg_public_key = &keys[0];
    let wg_private_key = &keys[1];

    let ret = ki.set_uci_var("network.wgExit.private_key", &wg_private_key);
    ret.expect("Failed to set UCI var! {:?}");
    let ret = ki.uci_commit();
    ret.expect("Failed to commit UCI changes!");

    //Mutates settings, intentional side effect
    SETTINGS.write().unwrap().network.wg_private_key = wg_private_key.to_string();
    SETTINGS.write().unwrap().network.wg_public_key = wg_public_key.to_string();

    Ok(())
}

fn linux_generate_mesh_ip(SETTINGS: Arc<RwLock<settings::RitaSettings>>) -> Result<(), Error> {
    let ki = KernelInterface {};
    let seed: String = thread_rng().gen_ascii_chars().take(50).collect();
    let mesh_ip = ipgen::ip(&seed, "fd::/120").unwrap();

    // Mutates Settings intentional side effect
    SETTINGS.write().unwrap().network.own_ip = mesh_ip;
    Ok(())
}

fn openwrt_generate_mesh_ip(SETTINGS: Arc<RwLock<settings::RitaSettings>>) -> Result<(), Error> {
    let ki = KernelInterface {};
    let seed = rand::thread_rng().gen::<[u8; 10]>();
    let mesh_ip = ipgen::ip(std::str::from_utf8(&seed)?, "fd::/120").unwrap();
    let ifaces = SETTINGS.read().unwrap().network.babel_interfaces.clone();
    let ifaces = ifaces.split(" ");

    // Mutates Settings intentional side effect
    SETTINGS.write().unwrap().network.own_ip = mesh_ip;

    for interface in ifaces {
        let identifier = "network.babel_".to_string() + interface;
        ki.set_uci_var(&identifier, &mesh_ip.to_string()).unwrap();
    }

    ki.uci_commit().unwrap();
    Ok(())
}

fn validate_wg_key(key: &str) -> bool {
    if key.len() != 44 || !key.ends_with("=") {
        false
    } else {
        true
    }
}

fn validate_mesh_ip(ip: &IpAddr) -> bool {
    if !ip.is_ipv6() || ip.is_unspecified() {
        false
    } else {
        true
    }
}

fn openwrt_validate_exit_setup() -> Result<(), Error> {
    Ok(())
}

fn linux_setup_exit_tunnel(SETTINGS: Arc<RwLock<settings::RitaSettings>>) -> Result<(), Error> {
    let ki = KernelInterface {};

    let details = SETTINGS
        .read()
        .unwrap()
        .exit_client
        .details
        .clone()
        .unwrap();

    ki.setup_wg_if_named("wg_exit").unwrap();
    ki.set_client_exit_tunnel_config(
        SocketAddr::new(
            SETTINGS.read().unwrap().exit_client.exit_ip,
            details.wg_exit_port,
        ),
        details.wg_public_key,
        SETTINGS.read().unwrap().network.wg_private_key_path.clone(),
        SETTINGS.read().unwrap().exit_client.wg_listen_port,
        details.internal_ip,
    );
    ki.set_route_to_tunnel(&"172.168.1.254".parse()?).unwrap();
    Ok(())
}

fn request_own_exit_ip(
    SETTINGS: Arc<RwLock<settings::RitaSettings>>,
) -> Result<ExitClientDetails, Error> {
    let exit_server = SETTINGS.read().unwrap().exit_client.exit_ip;
    let ident = althea_types::ExitIdentity {
        global: SETTINGS.read().unwrap().get_identity(),
        wg_port: SETTINGS.read().unwrap().exit_client.wg_listen_port.clone(),
    };

    let endpoint = format!(
        "http://[{}]:{}/setup",
        exit_server,
        SETTINGS.read().unwrap().exit_client.exit_registration_port
    );

    trace!("Sending exit setup request to {:?}", endpoint);
    let client = reqwest::Client::new();
    let response = client.post(&endpoint).json(&ident).send();

    let (exit_id, price): (LocalIdentity, u64) = response?.json()?;

    trace!("Got exit setup response {:?}", exit_id);

    Ok(ExitClientDetails {
        internal_ip: exit_id.local_ip,
        eth_address: exit_id.global.eth_address,
        wg_public_key: exit_id.global.wg_public_key,
        wg_exit_port: exit_id.wg_port,
        exit_price: price,
    })
}

// Replacement for the setup.ash file in althea firmware
fn openwrt_init(SETTINGS: Arc<RwLock<settings::RitaSettings>>) -> Result<(), Error> {
    let privkey = SETTINGS.read().unwrap().network.wg_private_key.clone();
    let pubkey = SETTINGS.read().unwrap().network.wg_public_key.clone();
    let mesh_ip = SETTINGS.read().unwrap().network.own_ip.clone();
    let our_exit_ip = SETTINGS.read().unwrap().exit_client.exit_ip.clone();

    request_own_exit_ip(SETTINGS.clone())?;
    trace!("Exit ip request exited");
    if !validate_wg_key(&privkey) || validate_wg_key(&pubkey) {
        openwrt_generate_and_set_wg_keys(SETTINGS.clone())?;
    }
    if !validate_mesh_ip(&mesh_ip) {
        openwrt_generate_mesh_ip(SETTINGS.clone())?;
    }
    if !our_exit_ip.is_ipv4() && !our_exit_ip.is_unspecified() {
        request_own_exit_ip(SETTINGS.clone())?;
    }
    Ok(())
}

fn linux_init(
    SETTINGS: Arc<RwLock<settings::RitaSettings>>,
    file_name: String,
) -> Result<(), Error> {
    let privkey = SETTINGS.read().unwrap().network.wg_private_key.clone();
    let pubkey = SETTINGS.read().unwrap().network.wg_public_key.clone();
    let mesh_ip = SETTINGS.read().unwrap().network.own_ip.clone();
    let our_exit_ip = SETTINGS.read().unwrap().exit_client.exit_ip.clone();

    if !validate_wg_key(&privkey) || validate_wg_key(&pubkey) {
        linux_generate_wg_keys(SETTINGS.clone()).expect("failed to generate wg keys");
    }
    if !validate_mesh_ip(&mesh_ip) {
        linux_generate_mesh_ip(SETTINGS.clone()).expect("failed to generate ip");
    }

    thread::spawn(move || {
        assert!(!our_exit_ip.is_ipv4());
        assert!(!our_exit_ip.is_unspecified());

        loop {
            let details = request_own_exit_ip(SETTINGS.clone());

            match details {
                Ok(details) => {
                    SETTINGS
                        .write()
                        .expect("can't write config!")
                        .exit_client
                        .details = Some(details);
                    SETTINGS
                        .read()
                        .expect("can't read config!")
                        .write(&file_name)
                        .expect("can't write config!");

                    linux_setup_exit_tunnel(SETTINGS.clone()).expect("can't set exit tunnel up!");

                    trace!("got exit details, exiting");
                    break;
                }
                Err(err) => {
                    trace!("got error back from requesting details, {:?}", err);
                }
            }
            thread::sleep(Duration::from_secs(5));
        }
    });

    Ok(())
}

pub fn init(platform: &str, file_name: &str, settings: Arc<RwLock<settings::RitaSettings>>) {
    match platform {
        "linux" => linux_init(settings.clone(), file_name.to_string()).unwrap(),
        _ => unimplemented!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_wg_key() {
        let good_key = "8BeCExnthLe5ou0EYec5jNqJ/PduZ1x2o7lpXJOpgXk=";
        let bad_key1 = "8BeCExnthLe5ou0EYec5jNqJ/PduZ1x2o7lpXJOpXk=";
        let bad_key2 = "look at me, I'm the same length as a key but";
        assert_eq!(validate_wg_key(&good_key), true);
        assert_eq!(validate_wg_key(&bad_key1), false);
        assert_eq!(validate_wg_key(&bad_key2), false);
    }

    #[test]
    fn test_generate_wg_key() {
        let mut ki = KernelInterface {};
        let keys = ki.create_wg_keypair().unwrap();
        let wg_public_key = &keys[0];
        let wg_private_key = &keys[1];
        assert_eq!(validate_wg_key(&wg_public_key), true);
        assert_eq!(validate_wg_key(&wg_private_key), true);
    }

    #[test]
    fn test_validate_mesh_ip() {
        let good_ip = "fd44:94c:41e2::9e6".parse::<IpAddr>().unwrap();
        let bad_ip = "192.168.1.1".parse::<IpAddr>().unwrap();
        assert_eq!(validate_mesh_ip(&good_ip), true);
        assert_eq!(validate_mesh_ip(&bad_ip), false);
    }
}
