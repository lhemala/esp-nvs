use crate::error::Error;
use crate::platform::Platform;
use crate::{Key, Nvs, raw};

pub trait Set<T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: T) -> Result<(), Error>;
}

impl<T: Platform> Set<bool> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: bool) -> Result<(), Error> {
        self.set_primitive(namespace, *key, raw::ItemType::U8, value as u64)
    }
}

impl<T: Platform> Set<u8> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: u8) -> Result<(), Error> {
        self.set_primitive(namespace, *key, raw::ItemType::U8, value as u64)
    }
}

impl<T: Platform> Set<u16> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: u16) -> Result<(), Error> {
        self.set_primitive(namespace, *key, raw::ItemType::U16, value as u64)
    }
}

impl<T: Platform> Set<u32> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: u32) -> Result<(), Error> {
        self.set_primitive(namespace, *key, raw::ItemType::U32, value as u64)
    }
}

impl<T: Platform> Set<u64> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: u64) -> Result<(), Error> {
        self.set_primitive(namespace, *key, raw::ItemType::U64, value)
    }
}

impl<T: Platform> Set<i8> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: i8) -> Result<(), Error> {
        self.set_primitive(
            namespace,
            *key,
            raw::ItemType::I8,
            value.cast_unsigned() as _,
        )
    }
}

impl<T: Platform> Set<i16> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: i16) -> Result<(), Error> {
        self.set_primitive(
            namespace,
            *key,
            raw::ItemType::I16,
            value.cast_unsigned() as _,
        )
    }
}

impl<T: Platform> Set<i32> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: i32) -> Result<(), Error> {
        self.set_primitive(
            namespace,
            *key,
            raw::ItemType::I32,
            value.cast_unsigned() as _,
        )
    }
}

impl<T: Platform> Set<i64> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: i64) -> Result<(), Error> {
        self.set_primitive(
            namespace,
            *key,
            raw::ItemType::I64,
            value.cast_unsigned() as _,
        )
    }
}

impl<T: Platform> Set<&str> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: &str) -> Result<(), Error> {
        self.set_str(namespace, *key, value)
    }
}

impl<T: Platform> Set<&[u8]> for Nvs<'_, T> {
    fn set(&mut self, namespace: &Key, key: &Key, value: &[u8]) -> Result<(), Error> {
        self.set_blob(namespace, *key, value)
    }
}
