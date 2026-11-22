//! Pure type relations shared by expression and pattern checking.

use super::super::semantic::Type;

pub(super) fn is_map_key_type(value: &Type) -> bool {
    match value {
        // Lists and structs are keys when their parts are, which a struct's
        // field types do not always show; the runtime checks them.
        Type::Bool
        | Type::Num
        | Type::Str
        | Type::Bytes
        | Type::Enum(..)
        | Type::Resource(_)
        | Type::Channel(_)
        | Type::List(None)
        | Type::Struct(..) => true,
        Type::List(Some(element)) => is_map_key_type(element) || is_dynamic_operation_type(element),
        Type::Tuple(elements) => elements
            .iter()
            .all(|element| is_map_key_type(element) || is_dynamic_operation_type(element)),
        Type::Union(members) => members.iter().all(is_map_key_type),
        _ => false,
    }
}

pub(super) fn is_dynamic_operation_type(value_type: &Type) -> bool {
    match value_type {
        Type::Unknown | Type::Any => true,
        Type::Union(members) => members.iter().any(is_dynamic_operation_type),
        _ => false,
    }
}

pub(super) fn type_intersection(left: &Type, right: &Type) -> Option<Type> {
    if let Type::Union(members) = left {
        return union_intersections(members, right);
    }
    if let Type::Union(members) = right {
        return union_intersections(members, left);
    }
    if left == right {
        return Some(left.clone());
    }
    match (left, right) {
        (Type::Struct(None, _), Type::Struct(Some(identity), arguments))
        | (Type::Struct(Some(identity), arguments), Type::Struct(None, _)) => {
            Some(Type::Struct(Some(identity.clone()), arguments.clone()))
        }
        // Two structs of one generic schema meet at it, whatever their type
        // arguments, which are erased.
        (Type::Struct(Some(left_identity), _), Type::Struct(Some(right_identity), _))
            if left_identity == right_identity =>
        {
            Some(left.clone())
        }
        // A map or struct may have a record type's fields; what matches is
        // the record type's values among it.
        (
            Type::Map(_) | Type::Struct(..) | Type::Record { .. },
            Type::Record { map: false, .. },
        ) if may_conform_to_record(left, right) => Some(right.clone()),
        (
            Type::Record { map: false, .. },
            Type::Map(_) | Type::Struct(..) | Type::Record { .. },
        ) if may_conform_to_record(right, left) => Some(left.clone()),
        // A tuple is a list of its elements.
        (Type::Tuple(_), Type::List(_)) if left.is_assignable_to(right) => Some(left.clone()),
        (Type::List(_), Type::Tuple(_)) if right.is_assignable_to(left) => Some(right.clone()),
        _ => None,
    }
}

pub(super) fn union_intersections(members: &[Type], other: &Type) -> Option<Type> {
    let intersections = members
        .iter()
        .filter_map(|member| type_intersection(member, other))
        .collect::<Vec<_>>();
    (!intersections.is_empty()).then(|| Type::union(intersections))
}

/// Whether some value of `value_type`, a map, struct, or record type, may
/// conform to `record`: whether each of its fields may be there with a value
/// of the field's type. A field's value the checker does not know may.
fn may_conform_to_record(value_type: &Type, record: &Type) -> bool {
    let Type::Record { fields, .. } = record else {
        return false;
    };
    fields.iter().all(|(name, expected)| {
        let actual = match value_type {
            Type::Record { map, .. } => match value_type.record_field(name) {
                Some(actual) => actual.clone(),
                None if *map => Type::Nil,
                None => return true,
            },
            Type::Map(Some((_, value))) => Type::union([value.as_ref().clone(), Type::Nil]),
            Type::Struct(Some(identity), arguments) => {
                let Some(schema) = super::super::semantic::schema_fields(identity, arguments)
                else {
                    return true;
                };
                let Some(actual) = schema.get(name) else {
                    return false;
                };
                actual.clone()
            }
            _ => return true,
        };
        is_dynamic_operation_type(&actual)
            || is_dynamic_operation_type(expected)
            || type_intersection(&actual, expected).is_some()
    })
}

pub(super) fn type_subtract(left: &Type, right: &Type) -> Option<Type> {
    // A record type covers only the members whose every value conforms.
    if let Type::Record { map: false, .. } = right {
        let members = match left {
            Type::Union(members) => members.clone(),
            other => vec![other.clone()],
        };
        let remaining = members
            .into_iter()
            .filter(|member| !member.is_assignable_to(right))
            .collect::<Vec<_>>();
        return (!remaining.is_empty()).then(|| Type::union(remaining));
    }
    if let Type::Union(members) = right {
        return members.iter().try_fold(left.clone(), |remaining, member| {
            type_subtract(&remaining, member)
        });
    }
    if let Type::Union(members) = left {
        let remaining = members
            .iter()
            .filter(|member| type_intersection(member, right).is_none())
            .cloned()
            .collect::<Vec<_>>();
        return (!remaining.is_empty()).then(|| Type::union(remaining));
    }
    if type_intersection(left, right).is_some() {
        None
    } else {
        Some(left.clone())
    }
}
