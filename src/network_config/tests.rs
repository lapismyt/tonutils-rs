//! Tests for network_config module

use super::*;
use std::net::Ipv4Addr;

#[test]
fn test_lite_server_address_from_i32() {
    let ip_int = 0x7F000001i32; // 127.0.0.1
    let addr = LiteServerAddress::from(ip_int);
    assert_eq!(*addr, Ipv4Addr::new(127, 0, 0, 1));
}

#[test]
fn test_lite_server_address_to_i32() {
    let addr = LiteServerAddress(Ipv4Addr::new(192, 168, 1, 1));
    let ip_int: i32 = addr.into();
    assert_eq!(ip_int, 0xC0A80101u32 as i32);
}

#[test]
fn test_lite_server_address_roundtrip() {
    let original_int = 0x08080808i32; // 8.8.8.8
    let addr = LiteServerAddress::from(original_int);
    let back_to_int: i32 = addr.into();
    assert_eq!(original_int, back_to_int);
}

#[test]
fn test_lite_server_address_deref() {
    let addr = LiteServerAddress(Ipv4Addr::new(10, 0, 0, 1));
    
    // Should be able to use Ipv4Addr methods through Deref
    assert!(addr.is_private());
    assert!(!addr.is_loopback());
}

#[test]
fn test_lite_server_address_deref_mut() {
    let mut addr = LiteServerAddress(Ipv4Addr::new(10, 0, 0, 1));
    
    // Should be able to mutate through DerefMut
    *addr = Ipv4Addr::new(127, 0, 0, 1);
    assert_eq!(*addr, Ipv4Addr::new(127, 0, 0, 1));
}

#[test]
fn test_config_public_key_ed25519() {
    let key = [42u8; 32];
    let pubkey = ConfigPublicKey::Ed25519 { key };
    
    match pubkey {
        ConfigPublicKey::Ed25519 { key: k } => {
            assert_eq!(k, [42u8; 32]);
        }
    }
}

#[test]
fn test_config_public_key_into_bytes() {
    let key = [99u8; 32];
    let pubkey = ConfigPublicKey::Ed25519 { key };
    
    let bytes: [u8; 32] = pubkey.into();
    assert_eq!(bytes, [99u8; 32]);
}

#[test]
fn test_config_lite_server_socket_addr() {
    let ip = LiteServerAddress::from(0x7F000001i32); // 127.0.0.1
    let server = ConfigLiteServer {
        ip,
        port: 8080,
        id: ConfigPublicKey::Ed25519 { key: [0; 32] },
    };
    
    let socket_addr = server.socket_addr();
    assert_eq!(socket_addr.ip(), &Ipv4Addr::new(127, 0, 0, 1));
    assert_eq!(socket_addr.port(), 8080);
}

#[test]
fn test_config_global_deserialization() {
    let json = r#"{
        "liteservers": [
            {
                "ip": 2130706433,
                "port": 46427,
                "id": {
                    "@type": "pub.ed25519",
                    "key": "peJTw/arlRfssgTuf9BMypJzqOi7SXEqSPSWiEw2U1M="
                }
            }
        ]
    }"#;
    
    let config: ConfigGlobal = json.parse().unwrap();
    
    assert_eq!(config.liteservers.len(), 1);
    assert_eq!(config.liteservers[0].port, 46427);
}

#[test]
fn test_config_global_multiple_servers() {
    let json = r#"{
        "liteservers": [
            {
                "ip": 2130706433,
                "port": 8001,
                "id": {
                    "@type": "pub.ed25519",
                    "key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                }
            },
            {
                "ip": 2130706434,
                "port": 8002,
                "id": {
                    "@type": "pub.ed25519",
                    "key": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="
                }
            }
        ]
    }"#;
    
    let config: ConfigGlobal = json.parse().unwrap();
    
    assert_eq!(config.liteservers.len(), 2);
    assert_eq!(config.liteservers[0].port, 8001);
    assert_eq!(config.liteservers[1].port, 8002);
}

#[test]
fn test_config_global_from_str_error() {
    let invalid_json = "{ invalid json }";
    let result = ConfigGlobal::from_str(invalid_json);
    
    assert!(result.is_err());
}

#[test]
fn test_config_lite_server_serialization_roundtrip() {
    let server = ConfigLiteServer {
        ip: LiteServerAddress::from(0x7F000001i32),
        port: 9000,
        id: ConfigPublicKey::Ed25519 { key: [55u8; 32] },
    };
    
    let json = serde_json::to_string(&server).unwrap();
    let deserialized: ConfigLiteServer = serde_json::from_str(&json).unwrap();
    
    assert_eq!(server.port, deserialized.port);
    let original_ip: i32 = server.ip.into();
    let deserialized_ip: i32 = deserialized.ip.into();
    assert_eq!(original_ip, deserialized_ip);
}

#[test]
fn test_config_global_serialization_roundtrip() {
    let config = ConfigGlobal {
        liteservers: vec![
            ConfigLiteServer {
                ip: LiteServerAddress::from(0x08080808i32),
                port: 443,
                id: ConfigPublicKey::Ed25519 { key: [1u8; 32] },
            }
        ],
    };
    
    let json = serde_json::to_string(&config).unwrap();
    let deserialized: ConfigGlobal = serde_json::from_str(&json).unwrap();
    
    assert_eq!(config.liteservers.len(), deserialized.liteservers.len());
    assert_eq!(config.liteservers[0].port, deserialized.liteservers[0].port);
}

#[test]
fn test_lite_server_address_localhost() {
    let addr = LiteServerAddress(Ipv4Addr::LOCALHOST);
    assert!(addr.is_loopback());
}

#[test]
fn test_lite_server_address_unspecified() {
    let addr = LiteServerAddress(Ipv4Addr::UNSPECIFIED);
    assert!(addr.is_unspecified());
}

#[test]
fn test_lite_server_address_broadcast() {
    let addr = LiteServerAddress(Ipv4Addr::BROADCAST);
    assert!(addr.is_broadcast());
}

#[test]
fn test_config_public_key_base64_deserialization() {
    let json = r#"{
        "@type": "pub.ed25519",
        "key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    }"#;
    
    let pubkey: ConfigPublicKey = serde_json::from_str(json).unwrap();
    
    match pubkey {
        ConfigPublicKey::Ed25519 { key } => {
            assert_eq!(key, [0u8; 32]);
        }
    }
}

#[test]
fn test_config_public_key_non_zero_base64() {
    let json = r#"{
        "@type": "pub.ed25519",
        "key": "//////////////////////////////////////////8="
    }"#;
    
    let pubkey: ConfigPublicKey = serde_json::from_str(json).unwrap();
    
    match pubkey {
        ConfigPublicKey::Ed25519 { key } => {
            assert_eq!(key, [255u8; 32]);
        }
    }
}

#[test]
fn test_lite_server_various_ports() {
    let ports = [80, 443, 8080, 3000, 65535];
    
    for port in ports {
        let server = ConfigLiteServer {
            ip: LiteServerAddress::from(0x7F000001i32),
            port,
            id: ConfigPublicKey::Ed25519 { key: [0; 32] },
        };
        
        assert_eq!(server.socket_addr().port(), port);
    }
}

#[test]
fn test_config_global_empty_servers() {
    let json = r#"{ "liteservers": [] }"#;
    let config: ConfigGlobal = json.parse().unwrap();
    
    assert_eq!(config.liteservers.len(), 0);
}

#[test]
fn test_ip_address_negative_i32() {
    // Test with negative i32 values (high bit set)
    let ip_int = -1i32; // 255.255.255.255
    let addr = LiteServerAddress::from(ip_int);
    assert_eq!(*addr, Ipv4Addr::new(255, 255, 255, 255));
}

#[test]
fn test_config_lite_server_debug_format() {
    let server = ConfigLiteServer {
        ip: LiteServerAddress::from(0x7F000001i32),
        port: 8080,
        id: ConfigPublicKey::Ed25519 { key: [42u8; 32] },
    };
    
    let debug_str = format!("{:?}", server);
    assert!(debug_str.contains("ConfigLiteServer"));
}

#[test]
fn test_config_public_key_debug_format() {
    let pubkey = ConfigPublicKey::Ed25519 { key: [99u8; 32] };
    let debug_str = format!("{:?}", pubkey);
    assert!(debug_str.contains("Ed25519"));
}

#[test]
fn test_config_global_clone() {
    let config = ConfigGlobal {
        liteservers: vec![
            ConfigLiteServer {
                ip: LiteServerAddress::from(0x7F000001i32),
                port: 8080,
                id: ConfigPublicKey::Ed25519 { key: [1u8; 32] },
            }
        ],
    };
    
    let cloned = config.clone();
    assert_eq!(config.liteservers.len(), cloned.liteservers.len());
    assert_eq!(config.liteservers[0].port, cloned.liteservers[0].port);
}
