//! Tests of the strict OBJ parser.

mod common;

use common::{box_obj, lid_obj, write_file};
use michelangelo_edit::obj::ObjError;
use michelangelo_edit::obj::{face_area, parse_obj, parse_obj_file};

#[test]
fn a_triangle_is_parsed() {
    let mesh = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").expect("the mesh is parsed");
    assert_eq!(mesh.vertex_count(), 3);
    assert_eq!(mesh.face_count(), 1);
    assert_eq!(mesh.triangle_count(), 1);
}

#[test]
fn a_quad_is_parsed() {
    let mesh =
        parse_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n").expect("the mesh is parsed");
    assert_eq!(mesh.face_count(), 1);
    assert_eq!(mesh.triangle_count(), 2);
}

#[test]
fn an_n_gon_is_parsed() {
    let mesh = parse_obj("v 0 0 0\nv 1 0 0\nv 2 1 0\nv 1 2 0\nv 0 2 0\nf 1 2 3 4 5\n")
        .expect("the mesh is parsed");
    assert_eq!(mesh.face_count(), 1);
    assert_eq!(mesh.triangle_count(), 3);
}

#[test]
fn the_bounding_box_is_computed() {
    let mesh = parse_obj(&box_obj(0.1)).expect("the mesh is parsed");
    assert_eq!(mesh.bbox.min, [-0.05, -0.05, -0.05]);
    assert_eq!(mesh.bbox.max, [0.05, 0.05, 0.05]);
    assert!(mesh.bbox.is_valid());
}

#[test]
fn the_geometry_keywords_are_ignored() {
    let text = "# a comment\no lid\ng group\ns off\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvn 0 0 1\nf 1 2 3\n";
    let mesh = parse_obj(text).expect("the mesh is parsed");
    assert_eq!(mesh.vertex_count(), 3);
    assert_eq!(mesh.face_count(), 1);
}

#[test]
fn the_texture_and_normal_indices_of_a_face_are_ignored() {
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1/1/1 2/2/2 3/3/3\n";
    let mesh = parse_obj(text).expect("the mesh is parsed");
    assert_eq!(mesh.face_count(), 1);
    assert_eq!(mesh.faces[0], vec![0, 1, 2]);
}

#[test]
fn a_non_finite_vertex_is_rejected() {
    let error =
        parse_obj("v 0 0 0\nv nan 0 0\nv 0 1 0\nf 1 2 3\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
    assert_eq!(error.line(), 2);
}

#[test]
fn an_infinite_vertex_is_rejected() {
    let error =
        parse_obj("v 0 0 0\nv inf 0 0\nv 0 1 0\nf 1 2 3\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
}

#[test]
fn an_out_of_range_index_is_rejected() {
    let error = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
}

#[test]
fn a_degenerate_face_is_rejected() {
    let error = parse_obj("v 0 0 0\nv 1 0 0\nv 2 0 0\nf 1 2 3\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
    assert!(error.message().contains("zero area"));
}

#[test]
fn a_face_that_repeats_a_vertex_is_rejected() {
    let error = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 1\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
}

#[test]
fn an_unknown_keyword_is_rejected() {
    let error = parse_obj("v 0 0 0\nl 1 2\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "OBJ_PARSE");
    assert!(error.message().contains("not supported"));
}

#[test]
fn a_face_with_two_vertices_is_rejected() {
    let error = parse_obj("v 0 0 0\nv 1 0 0\nf 1 2\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "OBJ_PARSE");
}

#[test]
fn a_vertex_without_coordinates_is_rejected() {
    let error = parse_obj("v 0 0\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "OBJ_PARSE");
}

#[test]
fn a_vertex_with_a_malformed_number_is_rejected() {
    let error = parse_obj("v 0 zero 0\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "OBJ_PARSE");
}

#[test]
fn the_index_zero_is_rejected() {
    let error = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "OBJ_PARSE");
}

#[test]
fn a_negative_index_is_relative_to_the_end() {
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
    let mesh = parse_obj(text).expect("the mesh is parsed");
    assert_eq!(mesh.faces[0], vec![0, 1, 2]);
}

#[test]
fn a_mesh_without_faces_is_rejected() {
    let error = parse_obj("v 0 0 0\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
}

#[test]
fn a_mesh_without_vertices_is_rejected() {
    let error = parse_obj("# nothing\n").expect_err("the mesh is refused");
    assert_eq!(error.code(), "GEOMETRY_INVALID");
}

#[test]
fn the_area_of_a_face_is_computed() {
    let mesh =
        parse_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n").expect("the mesh is parsed");
    let area = face_area(&mesh.vertices, &mesh.faces[0]);
    assert!((area - 1.0).abs() < 1.0e-9);
}

#[test]
fn the_digest_of_a_file_matches_its_bytes() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("lid.obj");
    let text = lid_obj(0.02);
    write_file(&path, &text);
    let file = parse_obj_file(&path).expect("the file is parsed");
    assert_eq!(
        file.sha256,
        michelangelo_input::sha256::sha256_hex(text.as_bytes())
    );
}

#[test]
fn a_missing_file_is_a_parse_error() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let error =
        parse_obj_file(&directory.path().join("missing.obj")).expect_err("the file is missing");
    assert_eq!(error.code(), "OBJ_PARSE");
}

#[test]
fn an_empty_box_is_parsed() {
    let mesh = parse_obj(&box_obj(0.2)).expect("the mesh is parsed");
    assert_eq!(mesh.vertex_count(), 8);
    assert_eq!(mesh.face_count(), 6);
    assert_eq!(mesh.triangle_count(), 12);
}

#[test]
fn the_error_carries_its_line() {
    let error: ObjError = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\nf 1 2 4\n")
        .expect_err("the mesh is refused");
    assert_eq!(error.line(), 5);
}
