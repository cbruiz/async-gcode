use super::{block_on, num, parse_all_and_count, to_gcode_comment, GCode};

#[test]
fn a_semicolon_is_followed_by_a_comment_until_the_end_of_the_line() {
    let msg = "Acceleration is defined as : δv/δt";
    let input = format!(";{}\n", msg);
    let input = input.bytes();
    let mut expected_output = to_gcode_comment(msg).to_vec();
    expected_output.push(Ok(GCode::Execute));
    assert_eq!(block_on(input), expected_output);

    let input = format!("g32 ;{}\n", msg);
    let input = input.bytes();
    let mut expected_output = vec![Ok(GCode::Word('g', num(32, 0)))];
    expected_output.extend_from_slice(&to_gcode_comment(msg));
    expected_output.push(Ok(GCode::Execute));
    assert_eq!(block_on(input), expected_output);
}

#[test]
fn a_trailing_comment_does_not_count_as_a_line() {
    let mut expected = to_gcode_comment(" one").to_vec();
    expected.push(Ok(GCode::Execute));
    expected.push(Ok(GCode::Word('g', num(1, 0))));
    expected.extend_from_slice(&to_gcode_comment(" two"));
    expected.push(Ok(GCode::Execute));
    expected.push(Ok(GCode::Word('g', num(2, 0))));
    expected.push(Ok(GCode::Execute));
    assert_eq!(
        parse_all_and_count("; one\nG1 ; two\r\nG2\r".bytes()),
        (expected, 3)
    );
}
