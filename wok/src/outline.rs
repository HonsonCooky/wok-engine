//! The scene-outline model: the pure tree data the explorer page renders.
//!
//! Separated from the egui layer so the explorer's content - what each row says and which kind
//! glyph it carries - is unit testable without a window, the same split the panels keep
//! everywhere (UI reads tested model functions, never derives display data inline). Lives beside
//! `crate::model` rather than inside it to keep both files within the size target.
//!
//! Row labels prefer the authored display name (`Placement::name`, the v2 schema addition) and
//! fall back to the generated `{prefab}_{id}` label, which stays the stable way to talk about an
//! unnamed placement. The kind glyph reads the dominant primitive of the placement's resolved
//! state: the shape the placeholder mostly is, which is what the eye should match against the
//! viewport.

use wok_scene::{ChunkCoord, InstanceId, Placement, Prefab, Primitive};

use crate::model::EditorModel;

/// One row of the scene tree: a placement under its chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementRow {
    pub id: InstanceId,
    /// What the row shows: the authored display name when present, else the generated label.
    pub label: String,
    /// The generated `{prefab}_{id}` label, always available (the details panel shows it dimmed
    /// under an authored name, and it is the rename field's placeholder).
    pub generated: String,
    pub prefab: String,
    /// The effective state name: the placement's explicit state, else the prefab's default.
    pub state: String,
    /// Dominant primitive of the resolved state's shapes, for the kind glyph; `None` when the
    /// prefab is missing or the state holds no shapes (mesh-only).
    pub kind: Option<Primitive>,
}

/// One chunk node of the scene tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkNode {
    pub coord: ChunkCoord,
    pub rows: Vec<PlacementRow>,
}

/// The generated label for a placement: prefab slug plus instance id (`oak_tree_42`).
pub fn generated_label(placement: &Placement) -> String {
    format!("{}_{}", placement.prefab.as_str(), placement.instance_id.0)
}

/// The scene-tree model: chunks in coordinate order, placements in authored order.
pub fn tree(model: &EditorModel) -> Vec<ChunkNode> {
    model
        .chunks
        .iter()
        .map(|(&coord, chunk)| ChunkNode {
            coord,
            rows: chunk.placements.iter().map(|p| row(model, p)).collect(),
        })
        .collect()
}

fn row(model: &EditorModel, placement: &Placement) -> PlacementRow {
    let prefab = model.prefabs.get(&placement.prefab);
    let state = placement
        .state
        .clone()
        .unwrap_or_else(|| prefab.map_or("default", |p| p.default_state.as_str()).to_string());
    let generated = generated_label(placement);
    PlacementRow {
        id: placement.instance_id,
        label: placement.name.clone().unwrap_or_else(|| generated.clone()),
        generated,
        prefab: placement.prefab.as_str().to_string(),
        kind: prefab.and_then(|p| dominant_primitive(p, &state)),
        state,
    }
}

/// The primitive a placement's resolved state mostly consists of: the most frequent primitive
/// among the state's shapes, ties broken toward the earliest-appearing one (the author listed it
/// first, so it is the shape they think of the prefab as). `None` for a missing state or a
/// shapeless (mesh-only) one.
pub fn dominant_primitive(prefab: &Prefab, state: &str) -> Option<Primitive> {
    let state = prefab.states.iter().find(|s| s.name == state)?;
    let mut counts: Vec<(Primitive, usize)> = Vec::new();
    for shape in &state.shapes {
        match counts.iter_mut().find(|(p, _)| *p == shape.primitive) {
            Some((_, n)) => *n += 1,
            None => counts.push((shape.primitive, 1)),
        }
    }
    // First-appearance order is the tie order: max_by_key keeps the earlier entry on ties only
    // with a stable scan, so scan manually.
    let mut best: Option<(Primitive, usize)> = None;
    for &(p, n) in &counts {
        if best.is_none_or(|(_, bn)| n > bn) {
            best = Some((p, n));
        }
    }
    best.map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Selection;
    use crate::sample;
    use wok_scene::{PrefabRef, PrefabState, Shape, SurfaceTag, Transform};

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    #[test]
    fn tree_matches_the_sample_fixture_structure() {
        let model = sample_model();
        let nodes = tree(&model);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].coord, ChunkCoord::new(0, 0));
        let labels: Vec<&str> = nodes[0].rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            ["crate_0", "crate_1", "crate_2", "boulder_3", "boulder_4", "pillar_5", "pillar_6", "marker_7"]
        );
        for row in &nodes[0].rows {
            assert_eq!(row.state, "default", "{}: explicit None resolves to the default", row.label);
            assert_eq!(row.label, row.generated, "unnamed rows show the generated label");
        }
    }

    #[test]
    fn a_display_name_replaces_the_generated_label_and_keeps_it_available() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: wok_scene::InstanceId(2) };
        assert!(model.rename(sel, "the big crate"));
        let nodes = tree(&model);
        let row = nodes[0].rows.iter().find(|r| r.id == sel.id).unwrap();
        assert_eq!(row.label, "the big crate");
        assert_eq!(row.generated, "crate_2", "the generated label stays available beside the name");
        assert_eq!(row.prefab, "crate");
    }

    #[test]
    fn row_kinds_follow_each_prefab_dominant_shape() {
        let model = sample_model();
        let nodes = tree(&model);
        for row in &nodes[0].rows {
            let expected = match row.prefab.as_str() {
                "crate" => Primitive::Cube,
                "boulder" => Primitive::Ellipsoid,
                "pillar" => Primitive::Cylinder,
                "marker" => Primitive::Capsule,
                other => panic!("unexpected prefab {other}"),
            };
            assert_eq!(row.kind, Some(expected), "{}", row.label);
        }
    }

    #[test]
    fn dominant_primitive_takes_the_majority_and_breaks_ties_by_first_appearance() {
        let shape = |primitive| Shape {
            primitive,
            transform: Transform::IDENTITY,
            surface: Some(SurfaceTag::new("s")),
            is_hitbox: true,
            is_visible: true,
        };
        let prefab = |shapes| Prefab {
            states: vec![PrefabState { name: "default".into(), shapes, mesh: None }],
            default_state: "default".into(),
        };

        let majority = prefab(vec![shape(Primitive::Cylinder), shape(Primitive::Cube), shape(Primitive::Cube)]);
        assert_eq!(dominant_primitive(&majority, "default"), Some(Primitive::Cube));

        let tied = prefab(vec![shape(Primitive::Ellipsoid), shape(Primitive::Cube)]);
        assert_eq!(dominant_primitive(&tied, "default"), Some(Primitive::Ellipsoid), "tie keeps the first");

        let empty = prefab(vec![]);
        assert_eq!(dominant_primitive(&empty, "default"), None, "mesh-only states have no kind");
        assert_eq!(dominant_primitive(&empty, "missing"), None, "unknown states have no kind");
    }

    #[test]
    fn a_placement_of_a_missing_prefab_still_rows_with_no_kind() {
        let mut model = sample_model();
        model.prefabs.remove(&PrefabRef::new("marker"));
        let nodes = tree(&model);
        let row = nodes[0].rows.iter().find(|r| r.prefab == "marker").unwrap();
        assert_eq!(row.kind, None);
        assert_eq!(row.state, "default", "a missing prefab still resolves a readable state");
    }
}
