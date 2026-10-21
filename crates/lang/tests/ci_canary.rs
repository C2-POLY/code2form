//! Deliberately broken: checks that a failing test fails the CI run.

#[test]
fn ci_fails_on_a_broken_test() {
    let two: i32 = "2".parse().unwrap();
    assert_eq!(two, 3, "deliberately broken test");
}
