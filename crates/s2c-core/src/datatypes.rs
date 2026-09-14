//! XSD datatype mapping to neutral schema value types, `s2c:datatype` overrides,
//! and static comparison with declared schema columns.

use oxrdf::NamedNodeRef;

use crate::schema::ValueType;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

/// What an `sh:datatype` constraint requires of each value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatatypeCheck {
    /// Value types able to hold values of the datatype.
    pub allowed: Vec<ValueType>,
    /// Inclusive integer bounds of derived integer datatypes such as `xsd:short`.
    pub range: Option<(i128, i128)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DatatypeError(pub String);

/// How a declared column relates to a datatype constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnStatus {
    /// Every value the column can hold satisfies the constraint.
    Guaranteed,
    /// The column type fits, but values may fall outside the datatype's range.
    NeedsRangeCheck,
    /// The column admits values of other types, so each value must be checked.
    NeedsCheck,
    /// No value the column can hold satisfies the constraint.
    Contradicted,
}

impl DatatypeCheck {
    /// The check for an `sh:datatype` value, or for an `s2c:datatype` override when given.
    // @lat: [[mapping#Datatypes]]
    pub fn for_datatype(
        datatype: NamedNodeRef<'_>,
        override_type: Option<&str>,
    ) -> Result<Self, DatatypeError> {
        if let Some(native) = override_type {
            return Ok(DatatypeCheck {
                allowed: vec![parse_override(native)?],
                range: None,
            });
        }
        let iri = datatype.as_str();
        if iri == RDF_LANG_STRING {
            return Err(DatatypeError(
                "rdf:langString is not supported: labeled property graphs have no language tags"
                    .into(),
            ));
        }
        let Some(local) = iri.strip_prefix(XSD) else {
            return Err(unsupported(iri));
        };
        type V = ValueType;
        let integers = || {
            vec![
                V::Int64,
                V::Int32,
                V::Int16,
                V::Int8,
                V::UInt64,
                V::UInt32,
                V::UInt16,
                V::UInt8,
            ]
        };
        let (allowed, range) = match local {
            "string" | "normalizedString" | "token" | "anyURI" => (vec![V::String], None),
            "boolean" => (vec![V::Boolean], None),
            "integer" => (integers(), None),
            "long" => (integers(), integer_bounds(&V::Int64)),
            "int" => (integers(), integer_bounds(&V::Int32)),
            "short" => (integers(), integer_bounds(&V::Int16)),
            "byte" => (integers(), integer_bounds(&V::Int8)),
            "unsignedLong" => (integers(), integer_bounds(&V::UInt64)),
            "unsignedInt" => (integers(), integer_bounds(&V::UInt32)),
            "unsignedShort" => (integers(), integer_bounds(&V::UInt16)),
            "unsignedByte" => (integers(), integer_bounds(&V::UInt8)),
            "nonNegativeInteger" => (integers(), Some((0, i128::MAX))),
            "positiveInteger" => (integers(), Some((1, i128::MAX))),
            "nonPositiveInteger" => (integers(), Some((i128::MIN, 0))),
            "negativeInteger" => (integers(), Some((i128::MIN, -1))),
            "decimal" => (vec![V::Decimal, V::Double, V::Float], None),
            "double" => (vec![V::Double, V::Float], None),
            "float" => (vec![V::Float, V::Double], None),
            "date" => (vec![V::Date], None),
            "dateTime" => (vec![V::ZonedDateTime, V::LocalDateTime], None),
            "dateTimeStamp" => (vec![V::ZonedDateTime], None),
            "time" => (vec![V::LocalTime, V::ZonedTime], None),
            "duration" | "dayTimeDuration" | "yearMonthDuration" => (vec![V::Duration], None),
            _ => return Err(unsupported(iri)),
        };
        Ok(DatatypeCheck { allowed, range })
    }

    /// Compares the check with a declared column type; list columns are judged by
    /// their element type unless the check itself expects a list.
    pub fn column_status(&self, column: &ValueType) -> ColumnStatus {
        let expects_list = self.allowed.iter().any(|t| matches!(t, ValueType::List(_)));
        match column {
            ValueType::Any => ColumnStatus::NeedsCheck,
            ValueType::List(element) if !expects_list => self.column_status(element),
            _ if self.allowed.contains(column) => match (self.range, integer_bounds(column)) {
                (Some((min, max)), Some((column_min, column_max)))
                    if column_min < min || column_max > max =>
                {
                    ColumnStatus::NeedsRangeCheck
                }
                _ => ColumnStatus::Guaranteed,
            },
            _ => ColumnStatus::Contradicted,
        }
    }
}

fn parse_override(native: &str) -> Result<ValueType, DatatypeError> {
    native
        .trim()
        .to_ascii_uppercase()
        .replace(' ', "_")
        .parse()
        .map_err(|_| {
            DatatypeError(format!(
                "s2c:datatype \"{native}\" is not a known value type (use schema type names such as STRING, INT64, LOCAL_DATETIME or LIST<STRING>)"
            ))
        })
}

fn unsupported(iri: &str) -> DatatypeError {
    DatatypeError(format!("sh:datatype <{iri}> is not supported"))
}

fn integer_bounds(value_type: &ValueType) -> Option<(i128, i128)> {
    Some(match value_type {
        ValueType::Int64 => (i64::MIN.into(), i64::MAX.into()),
        ValueType::Int32 => (i32::MIN.into(), i32::MAX.into()),
        ValueType::Int16 => (i16::MIN.into(), i16::MAX.into()),
        ValueType::Int8 => (i8::MIN.into(), i8::MAX.into()),
        ValueType::UInt64 => (0, u64::MAX.into()),
        ValueType::UInt32 => (0, u32::MAX.into()),
        ValueType::UInt16 => (0, u16::MAX.into()),
        ValueType::UInt8 => (0, u8::MAX.into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    fn xsd(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("{XSD}{local}"))
    }

    fn check(local: &str) -> DatatypeCheck {
        DatatypeCheck::for_datatype(xsd(local).as_ref(), None).unwrap()
    }

    fn list(element: ValueType) -> ValueType {
        ValueType::List(Box::new(element))
    }

    #[test]
    fn maps_xsd_datatypes_to_value_types() {
        assert_eq!(check("string").allowed, vec![ValueType::String]);
        assert_eq!(check("anyURI").allowed, vec![ValueType::String]);
        assert_eq!(check("boolean").allowed, vec![ValueType::Boolean]);
        assert_eq!(
            check("dateTime").allowed,
            vec![ValueType::ZonedDateTime, ValueType::LocalDateTime]
        );
        assert!(check("decimal").allowed.contains(&ValueType::Double));
        assert!(check("integer").allowed.contains(&ValueType::UInt64));
        assert_eq!(check("integer").range, None);
        assert_eq!(check("short").range, Some((-32768, 32767)));
        assert_eq!(check("unsignedByte").range, Some((0, 255)));
        assert_eq!(check("positiveInteger").range, Some((1, i128::MAX)));
    }

    #[test]
    fn rejects_language_strings_and_unsupported_datatypes() {
        let lang_string = NamedNode::new_unchecked(RDF_LANG_STRING);
        let message = DatatypeCheck::for_datatype(lang_string.as_ref(), None)
            .unwrap_err()
            .to_string();
        assert!(message.contains("no language tags"), "{message}");
        for iri in [
            xsd("gYear"),
            NamedNode::new_unchecked("http://example.org/Type"),
        ] {
            let message = DatatypeCheck::for_datatype(iri.as_ref(), None)
                .unwrap_err()
                .to_string();
            assert!(message.contains("is not supported"), "{message}");
        }
    }

    #[test]
    fn overrides_replace_the_xsd_mapping() {
        let override_check =
            |native: &str| DatatypeCheck::for_datatype(xsd("string").as_ref(), Some(native));
        assert_eq!(
            override_check("LOCAL DATETIME").unwrap(),
            DatatypeCheck {
                allowed: vec![ValueType::LocalDateTime],
                range: None
            }
        );
        assert_eq!(
            override_check("list<string>").unwrap().allowed,
            vec![list(ValueType::String)]
        );
        let message = override_check("VARCHAR").unwrap_err().to_string();
        assert!(message.contains("not a known value type"), "{message}");
    }

    #[test]
    fn compares_declared_columns() {
        use ColumnStatus::*;
        let cases = [
            ("string", ValueType::String, Guaranteed),
            ("integer", ValueType::String, Contradicted),
            ("integer", ValueType::UInt64, Guaranteed),
            ("short", ValueType::Int64, NeedsRangeCheck),
            ("short", ValueType::Int8, Guaranteed),
            ("short", ValueType::UInt8, Guaranteed),
            ("nonNegativeInteger", ValueType::Int64, NeedsRangeCheck),
            ("nonNegativeInteger", ValueType::UInt32, Guaranteed),
            ("dateTime", ValueType::ZonedDateTime, Guaranteed),
            ("date", ValueType::LocalDateTime, Contradicted),
            ("string", ValueType::Any, NeedsCheck),
            ("string", list(ValueType::String), Guaranteed),
            ("string", list(ValueType::Any), NeedsCheck),
            ("boolean", list(ValueType::String), Contradicted),
        ];
        for (datatype, column, expected) in cases {
            assert_eq!(
                check(datatype).column_status(&column),
                expected,
                "xsd:{datatype} on {column}"
            );
        }
        let list_override =
            DatatypeCheck::for_datatype(xsd("string").as_ref(), Some("LIST<STRING>")).unwrap();
        assert_eq!(
            list_override.column_status(&list(ValueType::String)),
            Guaranteed
        );
    }
}
