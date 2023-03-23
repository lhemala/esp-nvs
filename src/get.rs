//! The `Get<T>` trait and its implementation in this module allows providing a single generic,
//! overloaded function `get<T>()` for all supported types of the driver.

use crate::error::Error;
use crate::platform::Platform;
use crate::{Key, Nvs, raw};
use alloc::string::String;
use alloc::vec::Vec;

pub trait Get<T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<T, Error>;
}

impl<T: Platform> Get<bool> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<bool, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::U8)?;
        Ok(value != 0)
    }
}

impl<T: Platform> Get<u8> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<u8, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::U8)?;
        Ok(value as u8)
    }
}

impl<T: Platform> Get<u16> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<u16, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::U16)?;
        Ok(value as u16)
    }
}

impl<T: Platform> Get<u32> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<u32, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::U32)?;
        Ok(value as u32)
    }
}

impl<T: Platform> Get<u64> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<u64, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::U64)?;
        Ok(value)
    }
}

impl<T: Platform> Get<i8> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<i8, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::I8)?;
        Ok(value.cast_signed() as i8)
    }
}

impl<T: Platform> Get<i16> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<i16, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::I16)?;
        Ok(value.cast_signed() as i16)
    }
}

impl<T: Platform> Get<i32> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<i32, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::I32)?;
        Ok(value.cast_signed() as i32)
    }
}

impl<T: Platform> Get<i64> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<i64, Error> {
        let value = self.get_primitive(namespace, key, raw::ItemType::I64)?;
        Ok(value.cast_signed())
    }
}

impl<T: Platform> Get<String> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<String, Error> {
        self.get_string(namespace, key)
    }
}

impl<T: Platform> Get<Vec<u8>> for Nvs<'_, T> {
    fn get(&mut self, namespace: &Key, key: &Key) -> Result<Vec<u8>, Error> {
        self.get_blob(namespace, key)
    }
}
