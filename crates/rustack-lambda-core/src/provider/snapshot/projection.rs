//! Borrowed metadata serializer; original body serializers are never invoked.
//!
//! The source is the fixed live-store schema, not arbitrary input JSON. Field
//! paths are tracked through both derives and flattened maps. The projected JSON
//! is validated under the ordinary limits before a full owned DTO can exist.

use std::{
    fmt::{Debug, Display, Formatter, Result as FmtResult},
    io::{Error as IoError, ErrorKind, Result as IoResult, Write},
};

use serde::{
    Serialize, Serializer,
    ser::{
        Error as SerializeError, SerializeMap, SerializeSeq, SerializeStruct,
        SerializeStructVariant, SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Path {
    Root,
    Functions,
    Rows,
    Function,
    Versions,
    Version,
    Zip,
    Other,
}
impl Path {
    fn named(self, key: &str) -> Self {
        match (self, key) {
            (Self::Root, "functions") => Self::Functions,
            (Self::Functions, "functions") => Self::Rows,
            (Self::Function, "latest") => Self::Version,
            (Self::Function, "versions") => Self::Versions,
            (Self::Version, "zipBytesBase64") => Self::Zip,
            _ => Self::Other,
        }
    }
    fn element(self) -> Self {
        if self == Self::Rows {
            Self::Function
        } else {
            Self::Other
        }
    }
    fn key<K: Serialize + ?Sized, E: SerializeError>(self, key: &K) -> Result<Self, E> {
        if self == Self::Other {
            return Ok(Self::Other);
        }
        let mut bytes = KeyBytes(Vec::new());
        serde_json::to_writer(&mut bytes, key)
            .map_err(|_| E::custom("invalid archive metadata key"))?;
        Ok(match bytes.0.as_slice() {
            b"\"functions\"" => self.named("functions"),
            b"\"latest\"" => self.named("latest"),
            b"\"versions\"" => self.named("versions"),
            b"\"zipBytesBase64\"" => self.named("zipBytesBase64"),
            value
                if self == Self::Versions
                    && !value.is_empty()
                    && value.iter().all(u8::is_ascii_digit) =>
            {
                Self::Version
            }
            _ => Self::Other,
        })
    }
}
struct KeyBytes(Vec<u8>);
impl Debug for KeyBytes {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("KeyBytes(<redacted>)")
    }
}
impl Write for KeyBytes {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        let end = self
            .0
            .len()
            .checked_add(bytes.len())
            .ok_or(ErrorKind::FileTooLarge)?;
        // JSON escapes can encode one bounded key byte in six bytes, plus quotes.
        if end > 6 * 1024 + 2 {
            return Err(IoError::from(ErrorKind::FileTooLarge));
        }
        self.0
            .try_reserve(bytes.len())
            .map_err(|_| IoError::from(ErrorKind::OutOfMemory))?;
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

pub(super) struct Metadata<'a, T: ?Sized> {
    value: &'a T,
    path: Path,
}
impl<T: ?Sized> Debug for Metadata<'_, T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("Metadata(<borrowed>)")
    }
}
impl<'a, T: ?Sized> Metadata<'a, T> {
    pub(super) fn new(value: &'a T) -> Self {
        Self {
            value,
            path: Path::Root,
        }
    }
}
impl<T: Serialize + ?Sized> Serialize for Metadata<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(Project {
            inner: serializer,
            path: self.path,
        })
    }
}
struct Project<S> {
    inner: S,
    path: Path,
}
struct List<S> {
    inner: S,
    path: Path,
}
struct Fields<S> {
    inner: S,
    path: Path,
}
struct Map<S> {
    inner: S,
    path: Path,
    pending: Option<Path>,
}
macro_rules! private_debug {
    ($name:ident) => {
        impl<S> Debug for $name<S> {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
                formatter
                    .debug_struct(stringify!($name))
                    .field("path", &self.path)
                    .finish_non_exhaustive()
            }
        }
    };
}
private_debug!(Project);
private_debug!(List);
private_debug!(Fields);
private_debug!(Map);
macro_rules! scalar {
    ($name:ident, $type:ty) => {
        fn $name(self, value: $type) -> Result<Self::Ok, Self::Error> {
            self.inner.$name(value)
        }
    };
}
impl<S: Serializer> Serializer for Project<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = List<S::SerializeSeq>;
    type SerializeTuple = List<S::SerializeTuple>;
    type SerializeTupleStruct = List<S::SerializeTupleStruct>;
    type SerializeTupleVariant = List<S::SerializeTupleVariant>;
    type SerializeMap = Map<S::SerializeMap>;
    type SerializeStruct = Fields<S::SerializeStruct>;
    type SerializeStructVariant = Fields<S::SerializeStructVariant>;
    scalar!(serialize_bool, bool);
    scalar!(serialize_i8, i8);
    scalar!(serialize_i16, i16);
    scalar!(serialize_i32, i32);
    scalar!(serialize_i64, i64);
    scalar!(serialize_i128, i128);
    scalar!(serialize_u8, u8);
    scalar!(serialize_u16, u16);
    scalar!(serialize_u32, u32);
    scalar!(serialize_u64, u64);
    scalar!(serialize_u128, u128);
    scalar!(serialize_f32, f32);
    scalar!(serialize_f64, f64);
    scalar!(serialize_char, char);
    scalar!(serialize_str, &str);
    scalar!(serialize_bytes, &[u8]);
    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_none()
    }
    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_unit()
    }
    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_unit_struct(name)
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_unit_variant(name, index, variant)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_some(&Metadata {
            value,
            path: self.path,
        })
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_newtype_struct(
            name,
            &Metadata {
                value,
                path: self.path,
            },
        )
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.inner.serialize_newtype_variant(
            name,
            index,
            variant,
            &Metadata {
                value,
                path: Path::Other,
            },
        )
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(List {
            inner: self.inner.serialize_seq(len)?,
            path: self.path,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(List {
            inner: self.inner.serialize_tuple(len)?,
            path: self.path,
        })
    }
    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(List {
            inner: self.inner.serialize_tuple_struct(name, len)?,
            path: self.path,
        })
    }
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(List {
            inner: self
                .inner
                .serialize_tuple_variant(name, index, variant, len)?,
            path: Path::Other,
        })
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(Map {
            inner: self.inner.serialize_map(len)?,
            path: self.path,
            pending: None,
        })
    }
    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(Fields {
            inner: self.inner.serialize_struct(name, len)?,
            path: self.path,
        })
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(Fields {
            inner: self
                .inner
                .serialize_struct_variant(name, index, variant, len)?,
            path: Path::Other,
        })
    }
    fn collect_str<T: Display + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.inner.collect_str(value)
    }
    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}
macro_rules! list {
    ($trait:ident, $method:ident) => {
        impl<S: $trait> $trait for List<S> {
            type Ok = S::Ok;
            type Error = S::Error;
            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
                self.inner.$method(&Metadata {
                    value,
                    path: self.path.element(),
                })
            }
            fn end(self) -> Result<Self::Ok, Self::Error> {
                self.inner.end()
            }
        }
    };
}
list!(SerializeSeq, serialize_element);
list!(SerializeTuple, serialize_element);
list!(SerializeTupleStruct, serialize_field);
list!(SerializeTupleVariant, serialize_field);
macro_rules! fields {
    ($trait:ident) => {
        impl<S: $trait> $trait for Fields<S> {
            type Ok = S::Ok;
            type Error = S::Error;
            fn serialize_field<T: Serialize + ?Sized>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), Self::Error> {
                let path = self.path.named(key);
                if path == Path::Zip {
                    self.inner.serialize_field(key, &Option::<u8>::None)
                } else {
                    self.inner.serialize_field(key, &Metadata { value, path })
                }
            }
            fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
                self.inner.skip_field(key)
            }
            fn end(self) -> Result<Self::Ok, Self::Error> {
                self.inner.end()
            }
        }
    };
}
fields!(SerializeStruct);
fields!(SerializeStructVariant);
impl<S: SerializeMap> SerializeMap for Map<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.pending = Some(self.path.key::<_, Self::Error>(key)?);
        self.inner.serialize_key(key)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        let path = self
            .pending
            .take()
            .ok_or_else(|| Self::Error::custom("archive metadata value without key"))?;
        if path == Path::Zip {
            self.inner.serialize_value(&Option::<u8>::None)
        } else {
            self.inner.serialize_value(&Metadata { value, path })
        }
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.inner.end()
    }
}

#[cfg(test)]
mod tests;
