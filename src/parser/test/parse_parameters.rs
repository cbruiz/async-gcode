use crate::{
    types::expressions::{Expression, Operator},
    GCode,
};

#[cfg(feature = "optional-value")]
use crate::{types::RealValue, Error};

use super::{block_on, lit, num};

#[test]
fn parse_param_get() {
    let input = "G#-21.098\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::Word(
                'g',
                Expression(vec![
                    lit(-21098, 3).into(),
                    Operator::GetParameter.into()
                ])
                .into()
            )),
            Ok(GCode::Execute)
        ]
    );
}

#[test]
fn parse_nested_parameter_get() {
    let input = "G # # - 21 . 098 \n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::Word(
                'g',
                Expression(vec![
                    lit(-21098, 3).into(),
                    Operator::GetParameter.into(),
                    Operator::GetParameter.into()
                ])
                .into()
            )),
            Ok(GCode::Execute)
        ]
    )
}

#[test]
fn parse_param_set() {
    let input = "# 23.4 = -75.8 ".bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::ParameterSet(num(234, 1), num(-758, 1))),]
    );
}
#[test]
fn parse_param_set_with_nested_param_get() {
    let input = "# # 23.4 = # # -75.8 ".bytes();
    assert_eq!(
        block_on(input),
        &[Ok(GCode::ParameterSet(
            Expression(vec![
                lit(234, 1).into(),
                Operator::GetParameter.into()
            ])
            .into(),
            Expression(vec![
                lit(-758, 1).into(),
                Operator::GetParameter.into(),
                Operator::GetParameter.into()
            ])
            .into()
        )),]
    )
}

#[test]
#[cfg(feature = "optional-value")]
fn values_are_always_required_for_parameters_id() {
    let input = "G#\n".bytes();
    assert_eq!(block_on(input), &[Err(Error::UnexpectedByte(b'\n'))]);

    let input = "#=".bytes();
    assert_eq!(block_on(input), &[Err(Error::UnexpectedByte(b'='))]);
}

#[test]
#[cfg(feature = "optional-value")]
fn parameter_set_may_have_no_value() {
    let input = "#12= g\n#34=\n".bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::ParameterSet(
                lit(12, 0).into(),
                RealValue::None
            )),
            Ok(GCode::Word('g', RealValue::None)),
            Ok(GCode::Execute),
            Ok(GCode::ParameterSet(
                lit(34, 0).into(),
                RealValue::None
            )),
            Ok(GCode::Execute),
        ]
    );
}

#[test]
#[cfg(feature = "string-value")]
fn parse_param_with_string_index() {
    let input = r#"#"hello"="world" g#"hello""#.bytes();
    assert_eq!(
        block_on(input),
        &[
            Ok(GCode::ParameterSet(
                crate::Literal::from("hello".to_owned()).into(),
                crate::Literal::from("world".to_owned()).into()
            )),
            Ok(GCode::Word(
                'g',
                Expression(vec![
                    crate::Literal::from("hello".to_owned()).into(),
                    Operator::GetParameter.into()
                ])
                .into()
            )),
        ]
    );
}

#[test]
fn parameter_chains_are_bounded() {
    use crate::{Error, MAX_EXPRESSION_ITEMS};
    // The literal and its `GetParameter` operators.
    let longest = format!("G{}1\n", "#".repeat(MAX_EXPRESSION_ITEMS - 1));
    let mut expected = vec![lit(1, 0).into()];
    expected.extend(std::iter::repeat_n(
        Operator::GetParameter.into(),
        MAX_EXPRESSION_ITEMS - 1,
    ));
    assert_eq!(
        block_on(longest.bytes()),
        &[
            Ok(GCode::Word('g', Expression(expected).into())),
            Ok(GCode::Execute)
        ]
    );

    let too_long = format!("G{}1 G2\nG3\n", "#".repeat(MAX_EXPRESSION_ITEMS));
    assert_eq!(
        block_on(too_long.bytes()),
        &[
            Err(Error::InvalidExpression),
            Ok(GCode::Execute),
            Ok(GCode::Word('g', num(3, 0))),
            Ok(GCode::Execute),
        ]
    );
}
