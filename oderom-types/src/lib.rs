//! `oderom-types` -- Marco 1.2: the geometric type judgment over
//! `oderom-core` terms. See [`judgment::typecheck_monomial`] and
//! [`judgment::typecheck_polynomial`].

pub mod domain;
pub mod error;
pub mod judgment;

pub use domain::Domain;
pub use error::TypeError;
pub use judgment::{typecheck_monomial, typecheck_polynomial, ExprType};

#[cfg(test)]
mod tests {
    use super::*;
    use oderom_core::{
        AbstractIndex, Factor, Matching, Monomial, Polynomial, Registry, Scalar, SlotId, SlotSig,
        Variance,
    };
    use smallvec::SmallVec;

    fn registry_with_tm_and_v() -> (Registry, oderom_core::HeadId) {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let slots: SmallVec<[SlotSig; 4]> =
            smallvec::smallvec![SlotSig { bundle: tm, variance: Variance::Contra, dim: 4 }];
        let v = reg.declare_head("V", slots, vec![]).unwrap();
        (reg, v)
    }

    #[test]
    fn contracting_two_contravariant_tm_slots_is_a_type_error() {
        let (reg, v) = registry_with_tm_and_v();
        let factors: SmallVec<[Factor; 4]> =
            smallvec::smallvec![Factor { head: v }, Factor { head: v }];
        let contractions = Matching::try_new([(
            SlotId { factor: 0, slot: 0 },
            SlotId { factor: 1, slot: 0 },
        )])
        .unwrap();
        let m = Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &reg).unwrap();

        let err = typecheck_monomial(0, &m, &reg).unwrap_err();
        match &err {
            TypeError::IncompatibleContraction { left_slot, right_slot, .. } => {
                assert_eq!(*left_slot, SlotId { factor: 0, slot: 0 });
                assert_eq!(*right_slot, SlotId { factor: 1, slot: 0 });
            }
            other => panic!("expected IncompatibleContraction, got {other:?}"),
        }
        // The message names both offending slots in geometric language.
        let msg = err.to_string();
        assert!(msg.contains("TM"), "message should name the bundle: {msg}");
    }

    #[test]
    fn contracting_tm_with_its_dual_typechecks() {
        let mut reg = Registry::new();
        let m = reg.declare_manifold("M", 4).unwrap();
        let tm = reg.declare_bundle("TM", m, 4).unwrap();
        let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![
            SlotSig { bundle: tm, variance: Variance::Contra, dim: 4 },
            SlotSig { bundle: tm, variance: Variance::Co, dim: 4 },
        ];
        let mixed = reg.declare_head("W", slots, vec![]).unwrap();
        let factors: SmallVec<[Factor; 4]> = smallvec::smallvec![Factor { head: mixed }];
        let contractions = Matching::try_new([(
            SlotId { factor: 0, slot: 0 },
            SlotId { factor: 0, slot: 1 },
        )])
        .unwrap();
        let m = Monomial::try_new(Scalar::ONE, factors, contractions, vec![], &reg).unwrap();
        assert!(typecheck_monomial(0, &m, &reg).is_ok());
    }

    #[test]
    fn sum_with_mismatched_free_indices_is_a_type_error() {
        let (reg, v) = registry_with_tm_and_v();
        let term_a = Monomial::try_new(
            Scalar::ONE,
            smallvec::smallvec![Factor { head: v }],
            Matching::default(),
            vec![(SlotId { factor: 0, slot: 0 }, AbstractIndex::new("a"))],
            &reg,
        )
        .unwrap();
        let term_b = Monomial::try_new(
            Scalar::ONE,
            smallvec::smallvec![Factor { head: v }],
            Matching::default(),
            vec![(SlotId { factor: 0, slot: 0 }, AbstractIndex::new("b"))],
            &reg,
        )
        .unwrap();
        let poly = Polynomial { terms: vec![term_a, term_b] };

        let err = typecheck_polynomial(&poly, &reg).unwrap_err();
        assert!(matches!(err, TypeError::FreeIndexMismatch { term: 1, .. }));
    }

    #[test]
    fn sum_with_matching_free_indices_typechecks() {
        let (reg, v) = registry_with_tm_and_v();
        let term_a = Monomial::try_new(
            Scalar::ONE,
            smallvec::smallvec![Factor { head: v }],
            Matching::default(),
            vec![(SlotId { factor: 0, slot: 0 }, AbstractIndex::new("a"))],
            &reg,
        )
        .unwrap();
        let term_b = Monomial::try_new(
            Scalar::new(2, 1),
            smallvec::smallvec![Factor { head: v }],
            Matching::default(),
            vec![(SlotId { factor: 0, slot: 0 }, AbstractIndex::new("a"))],
            &reg,
        )
        .unwrap();
        let poly = Polynomial { terms: vec![term_a, term_b] };
        let ty = typecheck_polynomial(&poly, &reg).unwrap();
        assert_eq!(ty.free_signature.len(), 1);
    }
}
