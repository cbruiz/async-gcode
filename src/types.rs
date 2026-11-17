#[cfg(all(
    not(feature = "std"),
    any(feature = "parse-comments", feature = "string-value")
))]
use alloc::string::String;
#[cfg(feature = "float-f64")]
use core::f64;

#[derive(Debug)]
pub(crate) enum ParseResult<G, E> {
    Input(E),
    Parsing(crate::Error),
    Ok(G),
}
impl<O, E> From<core::result::Result<O, E>> for ParseResult<O, E> {
    fn from(f: core::result::Result<O, E>) -> Self {
        match f {
            Ok(o) => ParseResult::Ok(o),
            Err(e) => ParseResult::Input(e),
        }
    }
}

impl<O, E> From<crate::Error> for ParseResult<O, E> {
    fn from(f: crate::Error) -> Self {
        ParseResult::Parsing(f)
    }
}

impl<O, E> ParseResult<O, E> {
    /// The result of a parsing step: its error is a parsing one, not an input one.
    #[cfg(any(feature = "parse-comments", feature = "string-value"))]
    #[inline]
    pub(crate) fn from_parsing(res: Result<O, crate::Error>) -> Self {
        match res {
            Ok(o) => ParseResult::Ok(o),
            Err(e) => ParseResult::Parsing(e),
        }
    }
}

#[cfg(not(feature = "parse-comments"))]
pub type Comment = ();
#[cfg(feature = "parse-comments")]
pub type Comment = String;

#[derive(Debug, Clone, Copy, Default)]
pub struct DecimalRepr {
    integer: i32,
    scale: u8,
}

impl PartialEq for DecimalRepr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Brings the value with the lowest scale to the scale of the other one. When that
        // overflows, the scaled value is out of the i32 range and cannot equal the other one.
        let (low, high) = if self.scale <= other.scale {
            (self, other)
        } else {
            (other, self)
        };
        match 10i32.checked_pow(u32::from(high.scale.abs_diff(low.scale))) {
            Some(pow) => low.integer.checked_mul(pow) == Some(high.integer),
            // Only zero stays in the i32 range once scaled.
            None => low.integer == 0 && high.integer == 0,
        }
    }
}

impl DecimalRepr {
    pub const fn new(integer: i32, scale: u8) -> DecimalRepr {
        DecimalRepr {
            integer,
            scale,
        }
    }
    #[cfg(feature = "float-f64")]
    /// Convert from f64 with a maximum of 8 decimal positions
    pub fn from_f64(number: f64) -> DecimalRepr {
        // The cast truncates towards zero (and saturates), with no need for `f64::trunc`, which
        // `core` does not provide.
        let integer = (number * 100000000.0f64) as i32;
        DecimalRepr {
            integer,
            scale: 8,
        }
    }
    #[cfg(feature = "float-f64")]
    pub fn to_f64(&self) -> f64 {
        match 10u64.checked_pow(u32::from(self.scale)) {
            Some(pow) => self.integer as f64 / pow as f64,
            // A scale over 19: |value| < 2^31 / 10^20, i.e. 0 at the precision of a gcode.
            None => 0.0,
        }
    }

    #[inline]
    pub fn integer_part(&self) -> i32 {
        self.integer
    }

    #[inline]
    pub fn scale(&self) -> u8 {
        self.scale
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    RealNumber(DecimalRepr),
    #[cfg(feature = "string-value")]
    String(String),
}
impl Literal {
    pub fn as_decimal(&self) -> Option<DecimalRepr> {
        match self {
            Literal::RealNumber(rn) => Some(*rn),
            #[cfg(feature = "string-value")]
            _ => None,
        }
    }
    #[cfg(feature = "float-f64")]
    pub fn as_real_number(&self) -> Option<f64> {
        match self {
            Literal::RealNumber(rn) => Some(rn.to_f64()),
            #[cfg(feature = "string-value")]
            _ => None,
        }
    }
    #[cfg(feature = "string-value")]
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Literal::String(string) => Some(string),
            _ => None,
        }
    }
}

impl From<DecimalRepr> for Literal {
    fn from(from: DecimalRepr) -> Self {
        Self::RealNumber(from)
    }
}

impl From<i32> for Literal {
    fn from(from: i32) -> Self {
        Self::RealNumber(DecimalRepr::new(from, 0))
    }
}
impl TryFrom<u32> for Literal {

    type Error = crate::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(Self::RealNumber(
            DecimalRepr::new(
                i32::try_from(value)
                    .map_err(|_| crate::Error::InvalidNumberConversion)?, 0
            )
        ))
    }
}
#[cfg(feature = "float-f64")]
impl TryFrom<f64> for Literal {
    type Error = crate::Error;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        // As parser constraints: 5 decimals max
        let scaled_value = value * 100000.0;
        if scaled_value > f64::from(i32::MAX) || scaled_value < f64::from(i32::MIN) {
            Err(crate::Error::InvalidNumberConversion)
        } else {
            Ok(Self::RealNumber(
                DecimalRepr::new(scaled_value as i32, 5)
            ))
        }
    }
}

#[cfg(feature = "string-value")]
impl From<String> for Literal {
    fn from(from: String) -> Self {
        Self::String(from)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum RealValue {
    Literal(Literal),
    #[cfg(any(feature = "parse-parameters", feature = "parse-expressions"))]
    Expression(expressions::Expression),
    #[cfg(feature = "optional-value")]
    None,
}
impl Default for RealValue {
    fn default() -> Self {
        DecimalRepr::new(0, 0).into()
    }
}
impl<T: Into<Literal>> From<T> for RealValue {
    fn from(from: T) -> Self {
        RealValue::Literal(from.into())
    }
}

#[cfg(feature = "float-f64")]
impl TryFrom<f64> for RealValue {
    type Error = crate::Error;

    fn try_from(from: f64) -> Result<Self, Self::Error> {
        Ok(RealValue::Literal(from.try_into()?))
    }
}

#[cfg(any(feature = "parse-parameters", feature = "parse-expressions"))]
pub(crate) mod expressions {
    use super::{Literal, RealValue};
    use crate::Error;
    use either::Either;

    #[cfg(not(feature = "std"))]
    use alloc::vec::Vec;

    pub(crate) type ExprItem = Either<Operator, Literal>;
    pub(crate) type ExprInner = Vec<ExprItem>;

    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum OpType {
        Unary,
        Binary,
    }
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum Associativity {
        Left,
        #[cfg(feature = "parse-parameters")]
        Right,
    }
    #[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
    pub enum Precedence {
        Group1,
        Group2,
        Group3,
        #[cfg(feature = "parse-parameters")]
        Group4,
        Group5,
    }

    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum Operator {
        // Binary operators
        Add,
        Substract,
        Multiply,
        Divide,
        Power,

        And,
        Or,
        Xor,

        Modulus,

        // Unary operators
        Cos,
        Sin,
        Tan,
        ACos,
        ASin,
        ATan, // Atan is kind of binary

        Abs,
        Exp,
        Fix,
        Fup,
        Ln,
        Round,
        Sqrt,

        #[cfg(feature = "parse-parameters")]
        GetParameter,
    }
    impl Operator {
        pub fn op_type(&self) -> OpType {
            match self {
                Self::Add
                | Self::Substract
                | Self::Multiply
                | Self::Divide
                | Self::Modulus
                | Self::Power
                | Self::And
                | Self::Or
                | Self::Xor
                | Self::ATan => OpType::Binary,
                Self::Cos
                | Self::Sin
                | Self::Tan
                | Self::ACos
                | Self::ASin
                | Self::Abs
                | Self::Exp
                | Self::Fix
                | Self::Fup
                | Self::Ln
                | Self::Round
                | Self::Sqrt => OpType::Unary,
                #[cfg(feature = "parse-parameters")]
                Self::GetParameter => OpType::Unary,
            }
        }

        pub fn associativity(&self) -> Associativity {
            #[allow(clippy::match_single_binding)]
            match self {
                #[cfg(feature = "parse-parameters")]
                Self::GetParameter => Associativity::Right,
                _ => Associativity::Left,
            }
        }

        pub fn precedence(&self) -> Precedence {
            match *self {
                Self::Add | Self::Substract | Self::And | Self::Or | Self::Xor => {
                    Precedence::Group1
                }
                Self::Multiply | Self::Divide | Self::Modulus => Precedence::Group2,
                Self::Power => Precedence::Group3,
                #[cfg(feature = "parse-parameters")]
                Self::GetParameter => Precedence::Group4,
                Self::Cos
                | Self::Sin
                | Self::Tan
                | Self::ACos
                | Self::ASin
                | Self::ATan
                | Self::Abs
                | Self::Exp
                | Self::Fix
                | Self::Fup
                | Self::Ln
                | Self::Round
                | Self::Sqrt => Precedence::Group5,
            }
        }
    }

    #[derive(Debug, PartialEq, Clone)]
    pub struct Expression(pub(crate) ExprInner);
    impl Expression {
        /// Evaluates the expressions to a single literal. Math error may occur during the
        /// expression's resolution (e.g. division by 0).
        ///
        /// When `parse-parameters` is enabled, this method takes a closure as an argument.
        /// This closure is used to resolve parameters get.
        ///
        /// Not implemented yet: always returns [`Error::InvalidExpression`].
        pub fn evaluate(
            &self,
            #[cfg(feature = "parse-parameters")] _cbk: &mut dyn FnMut(Literal) -> Literal,
        ) -> Result<Literal, Error> {
            Err(Error::InvalidExpression)
        }
    }

    impl From<Operator> for Either<Operator, Literal> {
        fn from(from: Operator) -> Self {
            Self::Left(from)
        }
    }
    impl From<Literal> for Either<Operator, Literal> {
        fn from(from: Literal) -> Self {
            Self::Right(from)
        }
    }

    impl From<Expression> for Either<Literal, Expression> {
        fn from(from: Expression) -> Self {
            Self::Right(from)
        }
    }
    impl From<Literal> for Either<Literal, Expression> {
        fn from(from: Literal) -> Self {
            Self::Left(from)
        }
    }

    impl From<Expression> for RealValue {
        fn from(from: Expression) -> RealValue {
            RealValue::Expression(from)
        }
    }

    #[cfg(test)]
    mod test {
        // test plan for expressions:
        // TBD
    }
}

#[cfg(test)]
mod test {
    use super::DecimalRepr;

    #[test]
    fn decimal_repr_eq_compares_across_scales() {
        assert_eq!(DecimalRepr::new(3, 0), DecimalRepr::new(300, 2));
        assert_eq!(DecimalRepr::new(-300, 2), DecimalRepr::new(-3, 0));
        assert_ne!(DecimalRepr::new(3, 0), DecimalRepr::new(301, 2));
    }

    #[test]
    fn decimal_repr_eq_does_not_overflow() {
        // 300 * 10^8 and 10^12 are out of the i32 range.
        assert_ne!(DecimalRepr::new(300, 0), DecimalRepr::new(1, 8));
        assert_ne!(DecimalRepr::new(1, 12), DecimalRepr::new(1, 0));
        assert_eq!(DecimalRepr::new(0, 0), DecimalRepr::new(0, 12));
    }

    #[cfg(feature = "float-f64")]
    #[test]
    fn decimal_repr_to_f64_does_not_overflow() {
        assert_eq!(DecimalRepr::new(-15, 1).to_f64(), -1.5);
        assert_eq!(DecimalRepr::new(1, 25).to_f64(), 0.0);
    }
}
