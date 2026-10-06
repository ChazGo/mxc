// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Outbound network rule geometry for floor composition (design §4.5): CIDR
//! parsing and the overlap test that decides whether a catalog egress deny
//! conflicts with a required allow rule.

use crate::policy_store::json::Json;
use std::net::IpAddr;

/// One parsed CIDR: the address as a 128-bit value, its family, and prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    v6: bool,
    bits: u128,
    prefix: u8,
}

impl Cidr {
    fn width(self) -> u8 {
        if self.v6 {
            128
        } else {
            32
        }
    }

    fn mask(self) -> u128 {
        if self.prefix == 0 {
            0
        } else {
            let shift = u32::from(self.width() - self.prefix);
            (u128::MAX >> (128 - u32::from(self.width()))) & !((1u128 << shift) - 1)
        }
    }

    /// `other` equals this block or lies inside it.
    pub fn contains(self, other: Cidr) -> bool {
        self.v6 == other.v6
            && other.prefix >= self.prefix
            && (other.bits & self.mask()) == (self.bits & self.mask())
    }

    /// The smaller block when the two intersect (CIDR blocks either nest or
    /// are disjoint).
    fn intersection(self, other: Cidr) -> Option<Cidr> {
        if self.contains(other) {
            Some(other)
        } else if other.contains(self) {
            Some(self)
        } else {
            None
        }
    }

    fn halves(self) -> Option<[Cidr; 2]> {
        if self.prefix == self.width() {
            return None;
        }
        let prefix = self.prefix + 1;
        let low = self.bits & self.mask();
        let high = low | (1u128 << u32::from(self.width() - prefix));
        Some([
            Cidr {
                v6: self.v6,
                bits: low,
                prefix,
            },
            Cidr {
                v6: self.v6,
                bits: high,
                prefix,
            },
        ])
    }
}

/// Parses `address/prefix` (IPv4 or IPv6).
pub fn parse_cidr(value: &str) -> Option<Cidr> {
    let (address, prefix) = value.split_once('/')?;
    if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) || prefix.len() > 3 {
        return None;
    }
    let prefix: u8 = prefix.parse().ok()?;
    let (v6, bits) = match address.parse::<IpAddr>().ok()? {
        IpAddr::V4(v4) => (false, u128::from(u32::from(v4))),
        IpAddr::V6(v6) => (true, u128::from(v6)),
    };
    let cidr = Cidr { v6, bits, prefix };
    (prefix <= cidr.width()).then_some(cidr)
}

/// Whether `block` is entirely covered by the union of `excluded`.
fn covered(block: Cidr, excluded: &[Cidr]) -> bool {
    if excluded.iter().any(|e| e.contains(block)) {
        return true;
    }
    if !excluded.iter().any(|e| block.contains(*e)) {
        return false;
    }
    match block.halves() {
        Some([low, high]) => covered(low, excluded) && covered(high, excluded),
        None => false,
    }
}

/// One `to[]` peer: a block minus its exclusions.
#[derive(Clone, Debug)]
struct Peer {
    cidr: Cidr,
    except: Vec<Cidr>,
}

/// `None` for an omitted `to` (every destination in both families).
fn peers(rule: &Json) -> Option<Vec<Peer>> {
    let to = rule.get("to")?.as_array()?;
    Some(
        to.iter()
            .filter_map(|peer| {
                let cidr = parse_cidr(peer.get("cidr")?.as_str()?)?;
                let except = peer
                    .get("except")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().and_then(parse_cidr))
                    .collect();
                Some(Peer { cidr, except })
            })
            .collect(),
    )
}

fn everything(v6: bool) -> Peer {
    Peer {
        cidr: Cidr {
            v6,
            bits: 0,
            prefix: 0,
        },
        except: Vec::new(),
    }
}

fn destinations_intersect(allow: &Json, deny: &Json) -> bool {
    let all = || vec![everything(false), everything(true)];
    let allow = peers(allow).unwrap_or_else(all);
    let deny = peers(deny).unwrap_or_else(all);
    allow.iter().any(|a| {
        deny.iter().any(|d| match a.cidr.intersection(d.cidr) {
            None => false,
            Some(block) => {
                let mut excluded = a.except.clone();
                excluded.extend(&d.except);
                !covered(block, &excluded)
            }
        })
    })
}

/// A protocol and an inclusive port range (`None` for ICMP, which has none).
#[derive(Clone, Copy, Debug)]
struct Selector<'a> {
    protocol: &'a str,
    ports: Option<(u32, u32)>,
}

fn selectors(rule: &Json) -> Vec<Selector<'_>> {
    let Some(ports) = rule.get("ports").and_then(Json::as_array) else {
        return vec![Selector {
            protocol: "any",
            ports: None,
        }];
    };
    ports
        .iter()
        .map(|port| {
            let protocol = port.get("protocol").and_then(Json::as_str).unwrap_or("any");
            let start = port.get("port").and_then(Json::as_f64).map(|n| n as u32);
            let end = port.get("endPort").and_then(Json::as_f64).map(|n| n as u32);
            Selector {
                protocol,
                ports: start.map(|s| (s, end.unwrap_or(s))),
            }
        })
        .collect()
}

/// Whether two protocol/port selectors can match the same traffic. A
/// selector without ports matches every port, and ICMP (which has no ports)
/// as well when its protocol is `any`; a selector with ports never matches
/// ICMP.
fn selectors_intersect(left: Selector<'_>, right: Selector<'_>) -> bool {
    let protocol = match (left.protocol, right.protocol) {
        ("any", other) | (other, "any") => other,
        (l, r) if l == r => l,
        _ => return false,
    };
    match (left.ports, right.ports) {
        (Some((ls, le)), Some((rs, re))) => protocol != "icmp" && ls <= re && rs <= le,
        (Some(_), None) | (None, Some(_)) => protocol != "icmp",
        (None, None) => true,
    }
}

/// Whether a deny rule overlaps an allow rule: their destinations intersect
/// after `except` exclusions and their protocol/port selectors intersect.
pub fn rules_overlap(allow: &Json, deny: &Json) -> bool {
    destinations_intersect(allow, deny)
        && selectors(allow)
            .iter()
            .any(|a| selectors(deny).iter().any(|d| selectors_intersect(*a, *d)))
}

/// The full destination and port scope of a rule, for diagnostics.
pub fn describe_rule(rule: &Json) -> String {
    let to = match rule.get("to").and_then(Json::as_array) {
        None => "any destination".to_string(),
        Some(peers) => peers
            .iter()
            .map(|peer| {
                let cidr = peer.get("cidr").and_then(Json::as_str).unwrap_or("?");
                let except: Vec<&str> = peer
                    .get("except")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Json::as_str)
                    .collect();
                if except.is_empty() {
                    cidr.to_string()
                } else {
                    format!("{cidr} except {}", except.join(", "))
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    };
    let ports = match rule.get("ports").and_then(Json::as_array) {
        None => "any protocol and port".to_string(),
        Some(_) => selectors(rule)
            .iter()
            .map(|s| match s.ports {
                None => format!("{}/any port", s.protocol),
                Some((start, end)) if start == end => format!("{}/{start}", s.protocol),
                Some((start, end)) => format!("{}/{start}-{end}", s.protocol),
            })
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("[{to}; {ports}]")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(text: &str) -> Json {
        Json::parse(text).expect("valid JSON")
    }

    #[test]
    fn parses_cidrs() {
        assert!(parse_cidr("192.0.2.0/24").is_some());
        assert!(parse_cidr("2001:db8::/32").is_some());
        assert!(parse_cidr("192.0.2.0/33").is_none());
        assert!(parse_cidr("192.0.2.0").is_none());
        assert!(parse_cidr("192.0.2.0/+1").is_none());
        assert!(parse_cidr("host.example/24").is_none());
        let outer = parse_cidr("192.0.2.0/24").unwrap();
        assert!(outer.contains(parse_cidr("192.0.2.10/32").unwrap()));
        assert!(!outer.contains(parse_cidr("198.51.100.0/32").unwrap()));
        assert!(!outer.contains(parse_cidr("2001:db8::/128").unwrap()));
    }

    #[test]
    fn overlap_needs_destination_and_port_intersection() {
        let allow =
            rule(r#"{"to":[{"cidr":"192.0.2.10/32"}],"ports":[{"protocol":"tcp","port":443}]}"#);
        let cases = [
            (r#"{"to":[{"cidr":"192.0.2.0/24"}]}"#, true),
            (
                r#"{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":400,"endPort":500}]}"#,
                true,
            ),
            (
                r#"{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"udp","port":443}]}"#,
                false,
            ),
            (
                r#"{"to":[{"cidr":"192.0.2.0/24"}],"ports":[{"protocol":"tcp","port":22}]}"#,
                false,
            ),
            (r#"{"to":[{"cidr":"198.51.100.0/24"}]}"#, false),
            (
                r#"{"to":[{"cidr":"192.0.2.0/24","except":["192.0.2.0/28"]}]}"#,
                false,
            ),
            (
                r#"{"to":[{"cidr":"192.0.2.0/24","except":["192.0.2.0/29"]}]}"#,
                true,
            ),
            (r#"{"ports":[{"protocol":"any"}]}"#, true),
            (r#"{"to":[{"cidr":"2001:db8::/32"}]}"#, false),
            (r#"{"ports":[{"protocol":"icmp"}]}"#, false),
        ];
        for (deny, expected) in cases {
            assert_eq!(rules_overlap(&allow, &rule(deny)), expected, "{deny}");
        }
    }

    #[test]
    fn allow_exclusions_count() {
        let allow = rule(r#"{"to":[{"cidr":"192.0.2.0/24","except":["192.0.2.128/25"]}]}"#);
        assert!(!rules_overlap(
            &allow,
            &rule(r#"{"to":[{"cidr":"192.0.2.128/26"}]}"#)
        ));
        assert!(rules_overlap(
            &allow,
            &rule(r#"{"to":[{"cidr":"192.0.2.64/26"}]}"#)
        ));
    }

    #[test]
    fn describes_full_scope() {
        assert_eq!(
            describe_rule(&rule(
                r#"{"to":[{"cidr":"192.0.2.0/24","except":["192.0.2.1/32"]}],"ports":[{"protocol":"tcp","port":80,"endPort":90}]}"#
            )),
            "[192.0.2.0/24 except 192.0.2.1/32; tcp/80-90]"
        );
        assert_eq!(
            describe_rule(&rule("{}")),
            "[any destination; any protocol and port]"
        );
    }
}
