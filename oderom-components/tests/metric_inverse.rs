//! Structural tests for [`metric_inverse`]'s three-tier dispatch
//! (diagonal / block-decoupled / general), independent of any real
//! spacetime -- Kerr's own tests (`kerr.rs`) exercise the block path
//! against a genuine physical fixture; this file isolates the dispatch
//! logic itself with small, hand-built matrices where the expected
//! answer is checkable by hand, plus the one test that exists
//! specifically to prove block detection cannot silently misclassify a
//! coupled pair as decoupled (see `metric_block_structure`'s own doc
//! comment for why the union-find approach can't do that by
//! construction -- this test is the concrete demonstration).

use oderom_components::curvature::{metric_block_structure, metric_inverse, metric_inverse_diagonal, verify_metric_inverse};
use oderom_components::{Chart, ComponentTensor, Grid};
use oderom_core::{Perm, Registry, SignedPerm, SlotSig, Variance};
use oderom_expr::{normalize, Expr};
use smallvec::SmallVec;

fn symmetric_tensor(dim: usize, entries: &[(u8, u8, Expr)]) -> (Registry, Chart, ComponentTensor) {
    let mut registry = Registry::new();
    let manifold = registry.declare_manifold("M", dim as u32).unwrap();
    let tm = registry.declare_bundle("TM", manifold, dim as u32).unwrap();
    let co = SlotSig { bundle: tm, variance: Variance::Co, dim: dim as u32 };
    let slots: SmallVec<[SlotSig; 4]> = smallvec::smallvec![co, co];
    let head = registry.declare_head("g", slots, vec![SignedPerm::new(Perm::transposition(2, 0, 1), 1)]).unwrap();
    let coords: Vec<String> = (0..dim).map(|i| format!("x{i}")).collect();
    let chart = Chart::new(coords);

    let mut g = ComponentTensor::new(head);
    for (i, j, value) in entries {
        g.set(&registry, &[*i, *j], normalize(value)).unwrap();
    }
    (registry, chart, g)
}

/// Tier 1: an all-diagonal metric dispatches to exactly
/// `metric_inverse_diagonal`'s own result (same `Grid` content, not just
/// an equivalent one) -- the byte-for-byte non-regression guarantee this
/// round's whole premise depends on.
#[test]
fn diagonal_metric_dispatches_to_the_unchanged_fast_path() {
    let (registry, chart, g) = symmetric_tensor(
        3,
        &[(0, 0, Expr::var("p")), (1, 1, Expr::int(2) * Expr::var("q")), (2, 2, Expr::var("r").pow(2))],
    );

    let via_dispatch = metric_inverse(&registry, &chart, &g).unwrap();
    let via_direct = metric_inverse_diagonal(&registry, &chart, &g).unwrap();
    assert_eq!(via_dispatch.canonical_hash(), via_direct.canonical_hash(), "metric_inverse must dispatch a diagonal metric to metric_inverse_diagonal's own unchanged code path");

    verify_metric_inverse(&registry, &chart, &g, &via_dispatch).expect("g_ab * g^bc must equal delta^a_c");
}

/// Tier 2: a metric with one decoupled 2x2 block plus a singleton --
/// `metric_block_structure` finds exactly that partition, and the
/// resulting inverse satisfies `g*ginv=I`, cross-block entries included
/// (they must independently come out zero, not just be assumed so).
#[test]
fn block_decoupled_metric_partitions_correctly_and_inverts_correctly() {
    let (registry, chart, g) = symmetric_tensor(
        3,
        &[
            (0, 0, Expr::int(2)),
            (0, 1, Expr::int(1)),
            (1, 1, Expr::int(3)),
            (2, 2, Expr::var("s")),
        ],
    );

    let mut blocks = metric_block_structure(&registry, &chart, &g).unwrap();
    for b in &mut blocks {
        b.sort_unstable();
    }
    blocks.sort_by_key(|b| b[0]);
    assert_eq!(blocks, vec![vec![0, 1], vec![2]]);

    let ginv = metric_inverse(&registry, &chart, &g).unwrap();
    verify_metric_inverse(&registry, &chart, &g, &ginv).expect("g_ab * g^bc must equal delta^a_c");

    // Closed-form 2x2 check by hand: det([[2,1],[1,3]]) = 5, inverse =
    // [[3,-1],[-1,2]]/5.
    assert_eq!(ginv.get(&[0, 0]), normalize(&Expr::rational(3, 5)));
    assert_eq!(ginv.get(&[0, 1]), normalize(&Expr::rational(-1, 5)));
    assert_eq!(ginv.get(&[1, 1]), normalize(&Expr::rational(2, 5)));
    assert_eq!(ginv.get(&[0, 2]), Expr::zero());
    assert_eq!(ginv.get(&[1, 2]), Expr::zero());
    assert_eq!(ginv.get(&[2, 2]), normalize(&Expr::Pow(Box::new(Expr::var("s")), -1)));
}

/// Tier 3: a genuinely dense, hand-built 3x3 matrix -- no zero
/// off-diagonal entry at all, so `metric_block_structure` finds a single
/// block spanning every coordinate and the general adjugate/cofactor
/// path runs on the whole matrix.
#[test]
fn fully_general_hand_built_matrix_inverts_correctly() {
    let (registry, chart, g) = symmetric_tensor(
        3,
        &[
            (0, 0, Expr::int(2)),
            (0, 1, Expr::int(1)),
            (0, 2, Expr::int(1)),
            (1, 1, Expr::int(3)),
            (1, 2, Expr::int(1)),
            (2, 2, Expr::int(2)),
        ],
    );

    let blocks = metric_block_structure(&registry, &chart, &g).unwrap();
    assert_eq!(blocks.len(), 1, "a fully dense 3x3 metric must be one single block, got {blocks:?}");

    let ginv = metric_inverse(&registry, &chart, &g).unwrap();
    verify_metric_inverse(&registry, &chart, &g, &ginv).expect("g_ab * g^bc must equal delta^a_c");

    // det([[2,1,1],[1,3,1],[1,1,2]]) = 2*(3*2-1*1) - 1*(1*2-1*1) + 1*(1*1-3*1) = 2*5 - 1*1 + 1*(-2) = 10-1-2 = 7.
    let expected_det = Expr::int(7);
    // adjugate[0][0] = 3*2-1*1 = 5
    assert_eq!(ginv.get(&[0, 0]), normalize(&(Expr::int(5) * Expr::Pow(Box::new(expected_det.clone()), -1))));
}

/// The conservative-detection test this round's own safety requirement
/// exists for: a metric shaped to *look* like two decoupled 2x2 blocks
/// (`{0,1}` and `{2,3}`) but with one subtle, easy-to-miss nonzero
/// coupling entry (`g[1,2] = eps`, a bare free variable, nothing
/// numerically large or structurally loud about it) actually connecting
/// them. Union-find over "is this off-diagonal entry nonzero" has no
/// threshold to fool -- any nonzero entry merges its two blocks,
/// unconditionally -- so this must come back as ONE block of all four
/// coordinates, never the naive `[{0,1},{2,3}]` split.
#[test]
fn subtle_coupling_between_apparent_blocks_is_not_treated_as_decoupled() {
    let (registry, chart, g) = symmetric_tensor(
        4,
        &[
            (0, 0, Expr::int(2)),
            (0, 1, Expr::int(1)),
            (1, 1, Expr::int(2)),
            (1, 2, Expr::var("eps")), // the subtle coupling: connects the two "apparent" blocks
            (2, 2, Expr::int(2)),
            (2, 3, Expr::int(1)),
            (3, 3, Expr::int(2)),
        ],
    );

    let mut blocks = metric_block_structure(&registry, &chart, &g).unwrap();
    for b in &mut blocks {
        b.sort_unstable();
    }
    assert_eq!(blocks.len(), 1, "a subtle coupling between two apparent blocks must merge them into one block, got {blocks:?}");
    assert_eq!(blocks[0], vec![0, 1, 2, 3]);

    let ginv = metric_inverse(&registry, &chart, &g).unwrap();
    verify_metric_inverse(&registry, &chart, &g, &ginv).expect("the correct (single-block) inverse must satisfy g_ab * g^bc = delta^a_c");

    // Demonstrate the actual failure mode this test guards against:
    // naively inverting the two 2x2 blocks *as if* they were decoupled
    // (ignoring g[1,2]) produces a Grid that does NOT satisfy g*ginv=I --
    // proving the subtle coupling is not a cosmetic detail, it changes
    // the answer.
    let mut naive = Grid::new(4, 2);
    // det([[2,1],[1,2]]) = 3, inverse = [[2,-1],[-1,2]]/3, for BOTH blocks.
    for block_offset in [0u8, 2u8] {
        naive.set(&[block_offset, block_offset], normalize(&Expr::rational(2, 3)));
        naive.set(&[block_offset, block_offset + 1], normalize(&Expr::rational(-1, 3)));
        naive.set(&[block_offset + 1, block_offset], normalize(&Expr::rational(-1, 3)));
        naive.set(&[block_offset + 1, block_offset + 1], normalize(&Expr::rational(2, 3)));
    }
    let naive_result = verify_metric_inverse(&registry, &chart, &g, &naive);
    assert!(naive_result.is_err(), "the naive 'treat as two decoupled 2x2 blocks' inverse must FAIL g*ginv=I once the subtle coupling is real -- if this assertion fails, the test's own premise is wrong");
}
