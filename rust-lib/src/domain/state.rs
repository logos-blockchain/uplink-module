//! Decoding the referral program's account data.

use lee_core::account::ShardData;
use referral_core::{Participant, Registry, State};

use super::{DomainError, Result};

pub fn decode(data: &[u8]) -> Option<State> {
    let shard = ShardData::try_from(data.to_vec()).ok()?;
    State::decode(&shard)
}

pub fn registry(data: &[u8]) -> Result<Registry> {
    match decode(data) {
        Some(State::Registry(registry)) => Ok(registry),
        None if data.is_empty() => Ok(Registry::default()),
        _ => Err(DomainError::new("bad_registry", "the registry account holds other data")),
    }
}

pub fn participant(data: &[u8]) -> Result<Participant> {
    match decode(data) {
        Some(State::Participant(participant)) => Ok(participant),
        None if data.is_empty() => Err(DomainError::new("not_registered", "the participant is not registered")),
        _ => Err(DomainError::new("bad_participant", "the account holds other data")),
    }
}

/// A note (`Child` or `Credit`), or `None` for anything else, including a consumed note.
pub fn note(data: &[u8]) -> Option<State> {
    decode(data).filter(|state| matches!(state, State::Child { .. } | State::Credit { .. }))
}

/// An archived publication: (epoch, digest of its active set).
pub fn publication(data: &[u8]) -> Option<(u32, [u8; 32])> {
    match decode(data) {
        Some(State::Publication { epoch, active_digest }) => Some((epoch, active_digest)),
        _ => None,
    }
}

pub fn receipt_points(data: &[u8]) -> Option<u128> {
    match decode(data) {
        Some(State::CashOut { points }) => Some(points),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use referral_core::NodeId;

    use super::*;

    #[test]
    fn decodes_each_kind_of_account() {
        let node = NodeId::new([7; 32]);
        let registry = State::Registry(Registry { registered_nodes: BTreeSet::from([node]), epoch: 3, active_nodes: BTreeSet::new() });
        assert_eq!(super::registry(&registry.to_data().into_inner()).unwrap().epoch, 3);
        assert_eq!(super::registry(&[]).unwrap(), Registry::default());

        let child = State::Child { node, referrer: NodeId::new([8; 32]), since: 1 };
        assert_eq!(note(&child.to_data().into_inner()), Some(child));
        assert_eq!(note(&[]), None);
        assert_eq!(note(&registry.to_data().into_inner()), None);

        assert_eq!(receipt_points(&State::CashOut { points: 5 }.to_data().into_inner()), Some(5));
        let archived = State::Publication { epoch: 4, active_digest: [2; 32] };
        assert_eq!(publication(&archived.to_data().into_inner()), Some((4, [2; 32])));
        assert_eq!(participant(&[]).unwrap_err().code, "not_registered");
    }
}
