//! The `tangent` marker and the bundle a covariant derivative's index
//! lives in. Written as tests rather than exercised through the CLI
//! because slot *dimension* is not observable in rendered output --
//! which is also why the defect below sat undetected.

use oderom_core::{CoreError, Registry, SlotSig, Variance};
use smallvec::{smallvec, SmallVec};

fn co(bundle: oderom_core::BundleId, dim: u32) -> SlotSig {
    SlotSig { bundle, variance: Variance::Co, dim }
}

/// The defect Part 2 repairs, with the before-number measured in Part 0:
/// `head Z : E*, TM*` over `M` of dimension 4, with `E` of dimension 7,
/// used to give `∇` an index of dimension **7** because the signature
/// was copied from slot zero. Resolving through the manifold gives 4.
#[test]
fn a_non_tangent_first_slot_no_longer_decides_the_derivative_index() {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    let e = reg.declare_bundle("E", m, 7).unwrap();
    let tm = reg.declare_bundle_tangent("TM", m, 4, true).unwrap();
    let slots: SmallVec<[SlotSig; 4]> = smallvec![co(e, 7), co(tm, 4)];
    let z = reg.declare_head("Z", slots, vec![]).unwrap();

    let dz = reg.derivative_head(z, 1).unwrap();
    let last = *reg.head(dz).slots.last().unwrap();
    assert_eq!(last.dim, 4, "derivative index must live over the manifold, not in slot zero's bundle");
    assert_eq!(last.bundle, tm);
    assert_eq!(last.variance, Variance::Co);
}

/// Two bundles, one marked: `∇` resolves to the marked one even when it
/// is not slot zero.
#[test]
fn the_marked_bundle_wins_over_slot_order() {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    let e = reg.declare_bundle("E", m, 7).unwrap();
    let tm = reg.declare_bundle_tangent("TM", m, 4, true).unwrap();
    assert_eq!(reg.tangent_bundle(m).unwrap(), tm);
    let _ = e;
}

#[test]
fn two_bundles_and_none_marked_is_an_error_naming_both() {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    reg.declare_bundle("TM", m, 4).unwrap();
    reg.declare_bundle("E", m, 7).unwrap();
    let err = reg.tangent_bundle(m).unwrap_err();
    let CoreError::AmbiguousTangentBundle { manifold, candidates, marked } = &err else {
        panic!("expected AmbiguousTangentBundle, got {err:?}");
    };
    assert_eq!(manifold, "M");
    assert!(!marked);
    assert_eq!(candidates, &vec!["TM".to_string(), "E".to_string()]);
}

#[test]
fn two_bundles_both_marked_is_an_error() {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    reg.declare_bundle_tangent("TM", m, 4, true).unwrap();
    reg.declare_bundle_tangent("E", m, 7, true).unwrap();
    let err = reg.tangent_bundle(m).unwrap_err();
    let CoreError::AmbiguousTangentBundle { marked, .. } = &err else {
        panic!("expected AmbiguousTangentBundle, got {err:?}");
    };
    assert!(*marked, "the message must say more than one is marked, not that none is");
}

/// The default that keeps every existing `.od` file working unedited.
#[test]
fn one_bundle_unmarked_resolves_to_itself() {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    let tm = reg.declare_bundle("TM", m, 4).unwrap();
    assert_eq!(reg.tangent_bundle(m).unwrap(), tm);
}

/// Marking the only bundle is accepted and changes nothing.
#[test]
fn one_bundle_marked_is_accepted_and_behaves_the_same() {
    let mut reg = Registry::new();
    let m = reg.declare_manifold("M", 4).unwrap();
    let tm = reg.declare_bundle_tangent("TM", m, 4, true).unwrap();
    assert_eq!(reg.tangent_bundle(m).unwrap(), tm);
}
