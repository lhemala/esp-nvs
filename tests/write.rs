mod common;

mod set {
    use crate::common;
    use esp_nvs::Key;
    use esp_nvs::error::Error;
    use pretty_assertions::assert_eq;

    // TODO: test for writing namespace fails + cleanup

    #[test]
    fn primitives() {
        let mut flash = common::Flash::new(2);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        nvs.set(&Key::from_str("hello world"), &Key::from_str("bool"), false)
            .unwrap();
        assert_eq!(
            nvs.get::<bool>(&Key::from_str("hello world"), &Key::from_str("bool"))
                .unwrap(),
            false
        );

        nvs.set(&Key::from_str("hello world"), &Key::from_str("bool"), true)
            .unwrap();
        assert_eq!(
            nvs.get::<bool>(&Key::from_str("hello world"), &Key::from_str("bool"))
                .unwrap(),
            true
        );

        nvs.set(&Key::from_str("hello world"), &Key::from_str("u8"), 0xAAu8)
            .unwrap();
        assert_eq!(
            nvs.get::<u8>(&Key::from_str("hello world"), &Key::from_str("u8"))
                .unwrap(),
            0xAA
        );
        nvs.set(&Key::from_str("hello world"), &Key::from_str("i8"), -100i8)
            .unwrap();
        assert_eq!(
            nvs.get::<i8>(&Key::from_str("hello world"), &Key::from_str("i8"))
                .unwrap(),
            -100i8
        );

        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("u16"),
            0xAAAAu16,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<u16>(&Key::from_str("hello world"), &Key::from_str("u16"))
                .unwrap(),
            0xAAAAu16
        );
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("i16"),
            -30000i16,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<i16>(&Key::from_str("hello world"), &Key::from_str("i16"))
                .unwrap(),
            -30000i16
        );

        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("u32"),
            0xAAAAAAAAu32,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<u32>(&Key::from_str("hello world"), &Key::from_str("u32"))
                .unwrap(),
            0xAAAAAAAAu32
        );
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("i32"),
            -2000000000i32,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<i32>(&Key::from_str("hello world"), &Key::from_str("i32"))
                .unwrap(),
            -2000000000i32
        );

        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("u64"),
            0xAAAAAAAAAAAAAAAAu64,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<u64>(&Key::from_str("hello world"), &Key::from_str("u64"))
                .unwrap(),
            0xAAAAAAAAAAAAAAAAu64
        );

        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("i64"),
            -8000000000000000000i64,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<i64>(&Key::from_str("hello world"), &Key::from_str("i64"))
                .unwrap(),
            -8000000000000000000i64
        );
    }

    #[test]
    fn string() {
        let mut flash = common::Flash::new(2);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        nvs.set(&Key::from_str("hello world"), &Key::from_str("char"), "X")
            .unwrap();
        assert_eq!(
            nvs.get::<String>(&Key::from_str("hello world"), &Key::from_str("char"))
                .unwrap(),
            "X"
        );

        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("short str"),
            "short string",
        )
        .unwrap();
        assert_eq!(
            nvs.get::<String>(&Key::from_str("hello world"), &Key::from_str("short str"))
                .unwrap(),
            "short string"
        );

        let long_str = "long string spanning multiple items which is somewhat a different case";
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("long str"),
            long_str,
        )
        .unwrap();
        assert_eq!(
            nvs.get::<String>(&Key::from_str("hello world"), &Key::from_str("long str"))
                .unwrap(),
            long_str
        );
    }

    #[test]
    fn blob() {
        let mut flash = common::Flash::new(4);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        let tiny_blob: Vec<_> = (0u8..20).collect();
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("tiny blob"),
            tiny_blob.as_slice(),
        )
        .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("tiny blob"))
                .unwrap(),
            tiny_blob
        );

        let multi_page_blob: Vec<_> = (0u8..200).collect();
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("medium blob"),
            multi_page_blob.as_slice(),
        )
        .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("medium blob"))
                .unwrap(),
            multi_page_blob
        );

        let multi_page_blob: Vec<_> = (0u8..254).cycle().take(8192).collect();
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("multi page blob"),
            multi_page_blob.as_slice(),
        )
        .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(
                &Key::from_str("hello world"),
                &Key::from_str("multi page blob")
            )
            .unwrap(),
            multi_page_blob
        );
    }

    #[test]
    fn blob_replace_with_different_size() {
        let mut flash = common::Flash::new(4);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        let tiny_blob: Vec<_> = (0u8..20).collect();
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("tiny blob"),
            tiny_blob.as_slice(),
        )
        .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("tiny blob"))
                .unwrap(),
            tiny_blob
        );

        let tiny_blob: Vec<_> = (1u8..5).collect();
        nvs.set(
            &Key::from_str("hello world"),
            &Key::from_str("tiny blob"),
            tiny_blob.as_slice(),
        )
        .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("tiny blob"))
                .unwrap(),
            tiny_blob
        );
    }

    #[test]
    fn second_page_is_allocated() {
        let mut flash = common::Flash::new(3);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        // overflows into second page
        // 126 entries per page - 1 for namespace = 125
        for i in 0..126 {
            nvs.set(
                &Key::from_str("hello world"),
                &Key::from_str(&format!("{i}")),
                i,
            )
            .unwrap();
            assert_eq!(
                nvs.get::<u8>(
                    &Key::from_str("hello world"),
                    &Key::from_str(&format!("{i}"))
                )
                .unwrap(),
                i
            );
        }
    }

    #[test]
    fn primitive_overwrite_same_type() {
        let mut flash = common::Flash::new(2);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        for i in 0..10 {
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), i)
                .unwrap();
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                i,
                "in iteration {i}"
            );
        }
    }

    #[test]
    fn primitive_no_change() {
        let mut flash = common::Flash::new(2);

        // we need to drop nvs to be able to access flash.buf again
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), 1u8)
                .unwrap();
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                1
            );
        }

        let snapshot = flash.buf.clone();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), 1u8)
                .unwrap();
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                1
            );
        }

        assert_eq!(snapshot, flash.buf)
    }

    #[test]
    fn string_no_change() {
        let mut flash = common::Flash::new(2);

        let value = "hello";

        // we need to drop nvs to be able to access flash.buf again
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), value)
                .unwrap();
            assert_eq!(
                nvs.get::<String>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                value
            );
        }

        let snapshot = flash.buf.clone();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), value)
                .unwrap();
            assert_eq!(
                nvs.get::<String>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                value
            );
        }

        assert_eq!(snapshot, flash.buf)
    }

    #[test]
    fn blob_small_no_change() {
        let mut flash = common::Flash::new(2);

        let blob = (u8::MIN..u8::MAX).cycle().take(129).collect::<Vec<_>>();

        // we need to drop nvs to be able to access flash.buf again
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("hello world"),
                &Key::from_str("val"),
                blob.as_slice(),
            )
            .unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                blob
            );
        }

        let snapshot = flash.buf.clone();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("hello world"),
                &Key::from_str("val"),
                blob.as_slice(),
            )
            .unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                blob
            );
        }

        assert_eq!(snapshot, flash.buf)
    }

    #[test]
    fn blob_large_no_change() {
        let mut flash = common::Flash::new(3);

        let blob = (u8::MIN..u8::MAX).cycle().take(256).collect::<Vec<_>>();

        // we need to drop nvs to be able to access flash.buf again
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("hello world"),
                &Key::from_str("val"),
                blob.as_slice(),
            )
            .unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                blob
            );
        }

        let snapshot = flash.buf.clone();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("hello world"),
                &Key::from_str("val"),
                blob.as_slice(),
            )
            .unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("val"))
                    .unwrap(),
                blob
            );
        }

        assert_eq!(snapshot, flash.buf)
    }

    #[test]
    fn namespace_still_fits_but_item_not_so_new_page_is_allocated() {
        let mut flash = common::Flash::new(3);

        {
            // we fill the partition so that a only single entry still fits
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            // 126 entries per page - 1 for namespace = 125
            for i in 0u8..124 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), i)
                    .unwrap();
            }
        }

        // last item on first page is unused
        assert_eq!(flash.buf[4096 - 32..4096], vec![0xffu8; 32]);

        // second page is still uninitialized
        assert_eq!(flash.buf[4096..4096 * 2], vec![0xffu8; 4096]);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("ns2"),
                &Key::from_str("another item"),
                u64::MIN,
            )
            .unwrap();
            assert_eq!(
                nvs.get::<u64>(&Key::from_str("ns2"), &Key::from_str("another item"))
                    .unwrap(),
                u64::MIN
            );
        }

        // last item on first page is unused
        assert_ne!(flash.buf[4096 - 32..4096], vec![0xffu8; 32]);

        // second page is now in use
        assert_ne!(flash.buf[4096..4096 * 2], vec![0xffu8; 4096]);
    }

    #[test]
    fn string_not_fitting_into_active_page() {
        let mut flash = common::Flash::new(3);

        {
            // we fill the partition so that a only 4 entries still fit
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            for i in 0u8..121 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), i)
                    .unwrap();
            }
        }

        // last 4 item on first page are unused
        assert_eq!(flash.buf[4096 - (32 * 4)..4096], vec![0xffu8; 32 * 4]);

        // second page is still uninitialized
        assert_eq!(flash.buf[4096..4096 * 2], vec![0xffu8; 4096]);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let long_string = "X".repeat(100);
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("another item"),
                long_string.as_str(),
            )
            .unwrap();
            assert_eq!(
                nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("another item"))
                    .unwrap(),
                long_string
            );
        }

        // last 4 item on first page are still unused
        assert_eq!(flash.buf[4096 - (32 * 4)..4096], vec![0xffu8; 32 * 4]);

        // second page is now in use
        assert_ne!(flash.buf[4096..4096 * 2], vec![0xffu8; 4096]);
    }

    #[test]
    fn propagate_flash_full_error() {
        let mut flash = common::Flash::new(2);

        {
            // we fill the partition so that it's filled
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            // 126 entries per page - 1 for namespace = 125
            for i in 0u8..125 {
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("item_{i}")),
                    i,
                )
                .unwrap();
            }
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.set::<u8>(&Key::from_str("ns1"), &Key::from_str("item_125"), 1);
        assert_eq!(result, Err(Error::FlashFull));
    }
}

mod delete {
    use crate::common;
    use esp_nvs::error::Error;
    use esp_nvs::{EntryStatistics, Key, NvsStatistics, PageStatistics};
    use pretty_assertions::assert_eq;

    #[test]
    fn primitive() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("primitive"), 123)
                .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("primitive"))
                .unwrap();

            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("primitive"));
            assert!(result.is_err());

            assert_eq!(result.err().unwrap(), Error::KeyNotFound);
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.get::<u32>(&Key::from_str("ns1"), &Key::from_str("primitive"));
        assert!(result.is_err());

        assert_eq!(result.err().unwrap(), Error::KeyNotFound);
    }

    #[test]
    fn string() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let long_string = "X".repeat(100);
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("long string"),
                long_string.as_str(),
            )
            .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("long string"))
                .unwrap();

            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("long string"));
            assert!(result.is_err());

            assert_eq!(result.err().unwrap(), Error::KeyNotFound);
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("long string"));
        assert!(result.is_err());

        assert_eq!(result.err().unwrap(), Error::KeyNotFound);
    }

    #[test]
    fn blob_small() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let blob = (u8::MIN..u8::MAX).cycle().take(128).collect::<Vec<_>>();
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob.as_slice(),
            )
            .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap();

            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("blob"));
            assert!(result.is_err());

            assert_eq!(result.err().unwrap(), Error::KeyNotFound);
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("blob"));
        assert!(result.is_err());

        assert_eq!(result.err().unwrap(), Error::KeyNotFound);
    }

    #[test]
    fn blob_large() {
        let mut flash = common::Flash::new(4);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let blob = (u8::MIN..u8::MAX)
                .cycle()
                .take(4096 * 2)
                .collect::<Vec<_>>();
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob.as_slice(),
            )
            .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap();

            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("blob"));
            assert!(result.is_err());

            assert_eq!(result.err().unwrap(), Error::KeyNotFound);
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("blob"));
        assert!(result.is_err());

        assert_eq!(result.err().unwrap(), Error::KeyNotFound);

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 1,
                    active: 1,
                    full: 2,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 0,
                        written: 1,
                        erased: 125,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 0,
                        written: 0,
                        erased: 126,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 117,
                        written: 0,
                        erased: 9,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    }
                ],
                entries_overall: EntryStatistics {
                    empty: 243,
                    written: 1,
                    erased: 260,
                    illegal: 0,
                },
            }
        );
    }

    #[test]
    fn nonexisting_key() {
        let mut flash = common::Flash::new(1);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.delete(&Key::from_str("ns1"), &Key::from_str("my_key"));

        assert!(result.is_ok());
    }
}

mod overwrite {
    use crate::common;
    use esp_nvs::error::Error::{FlashError, KeyNotFound};
    use esp_nvs::{EntryStatistics, Key, NvsStatistics, PageStatistics};
    use pretty_assertions::assert_eq;

    #[test]
    fn primitive_overwrites_primitive() {
        let mut flash = common::Flash::new(2);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("my_primitive"), 42u8)
            .unwrap();

        nvs.set(
            &Key::from_str("ns1"),
            &Key::from_str("my_primitive"),
            1337u16,
        )
        .unwrap();

        assert_eq!(
            nvs.get::<u16>(&Key::from_str("ns1"), &Key::from_str("my_primitive"))
                .unwrap(),
            1337
        );

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 1,
                    active: 1,
                    full: 0,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 123,
                        written: 2,
                        erased: 1,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    }
                ],
                entries_overall: EntryStatistics {
                    empty: 249,
                    written: 2,
                    erased: 1,
                    illegal: 0,
                },
            }
        );
    }

    #[test]
    fn primitive_ensure_write_before_delete() {
        let mut flash = common::Flash::new_with_fault(2, 10);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), 1u8)
                .unwrap();

            // The fault is injected here right before the deletion of the old value
            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), 2u8),
                Err(FlashError)
            );
        }

        flash.disable_faults();

        // The new value should be readable even though deletion of the old value failed
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("item"))
                .unwrap(),
            2
        );
    }

    #[test]
    fn blob_overwrites_blob() {
        let mut flash = common::Flash::new(6);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        let blob = (u8::MIN..u8::MAX)
            .cycle()
            .take(4096 * 2)
            .collect::<Vec<_>>();

        println!("write initial value");
        nvs.set(
            &Key::from_str("ns1"),
            &Key::from_str("blob"),
            blob.as_slice(),
        )
        .unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob
        );

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 3,
                    active: 1,
                    full: 2,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 0,
                        written: 126,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 0,
                        written: 126,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 117,
                        written: 9,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                ],
                entries_overall: EntryStatistics {
                    empty: 369 + 126,
                    written: 261,
                    erased: 0,
                    illegal: 0,
                },
            }
        );

        println!("overwrite first time");
        let blob = (u8::MIN..u8::MAX)
            .rev()
            .cycle()
            .take(4096 * 2)
            .collect::<Vec<_>>();
        nvs.set(
            &Key::from_str("ns1"),
            &Key::from_str("blob"),
            blob.as_slice(),
        )
        .unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob
        );

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 1,
                    active: 1,
                    full: 4,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 0,
                        written: 1,
                        erased: 125,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 0,
                        written: 0,
                        erased: 126,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 0,
                        written: 117,
                        erased: 9,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 0,
                        written: 126,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 109,
                        written: 17,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                ],
                entries_overall: EntryStatistics {
                    empty: 109 + 126,
                    written: 261,
                    erased: 260,
                    illegal: 0,
                },
            }
        );

        for i in 0..10 {
            println!("overwrite another time: {i}");

            let blob = if i % 2 == 0 {
                (u8::MIN..u8::MAX)
                    .cycle()
                    .take(4096 * 2)
                    .collect::<Vec<_>>()
            } else {
                (u8::MIN..u8::MAX)
                    .rev()
                    .cycle()
                    .take(4096 * 2)
                    .collect::<Vec<_>>()
            };

            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob.as_slice(),
            )
            .unwrap();
        }

        // TODO smaller override bigger

        // TODO bigger override smaller
    }

    #[test]
    fn blob_is_written_partially() {
        // fail_after_operations is the highest value that makes writing the blob fail.
        // That means that already parts of blob have been written to flash but the old but the
        // chunk index is missing -> there are orphaned chunks on the flash.
        let mut flash = common::Flash::new_with_fault(3, 14);

        let blob = (u8::MIN..u8::MAX).cycle().take(4096).collect::<Vec<_>>();
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("blob"),
                    blob.as_slice()
                ),
                Err(FlashError)
            );
        }
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob")),
            Err(KeyNotFound)
        );

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 1,
                    active: 1,
                    full: 1,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 0,
                        written: 1,
                        erased: 125,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 121,
                        written: 0,
                        erased: 5,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                ],
                entries_overall: EntryStatistics {
                    empty: 247,
                    written: 1,
                    erased: 130,
                    illegal: 0,
                },
            }
        );
    }

    #[test]
    fn blob_overwrites_blob_atomicity_fail_to_write_index() {
        // fail_after_operations is the highest value that makes writing the changed block fail.
        // That means that already parts of blob_changed have been written to flash but the old
        // chunk_index has not been marked as erased yet.
        let mut flash = common::Flash::new_with_fault(4, 23);

        let blob_initial = (u8::MIN..u8::MAX).cycle().take(4096).collect::<Vec<_>>();
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob_initial.as_slice(),
            )
            .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob_initial
            );

            let blob_changed = (u8::MIN..u8::MAX)
                .rev()
                .cycle()
                .take(4096)
                .collect::<Vec<_>>();

            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("blob"),
                    blob_changed.as_slice()
                ),
                Err(FlashError)
            );
        }
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob_initial
        );
    }

    #[test]
    fn blob_overwrites_blob_atomicity_fail_to_delete_old() {
        // fail_after_operations is the highest value that makes deleting the old, overwritten block fail.
        let mut flash = common::Flash::new_with_fault(5, 39);

        // a page has 126 entries
        // the first page contains the namespace, the header for the blob_data and the first 124*32 bytes
        // the seconds page contains the blob_data header, 124*32 bytes of data and the blob_index entry
        let blob_initial = (u8::MIN..u8::MAX)
            .cycle()
            .take(124 * 32 + 124 * 32)
            .collect::<Vec<_>>();
        let blob_changed = (u8::MIN..u8::MAX)
            .rev()
            .cycle()
            .take(125 * 32 + 124 * 32)
            .collect::<Vec<_>>();
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob_initial.as_slice(),
            )
            .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob_initial
            );

            println!("{:?}", nvs.statistics().unwrap());

            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("blob"),
                    blob_changed.as_slice()
                ),
                Err(FlashError)
            );
        }
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob_changed
        );
    }

    #[test]
    fn blob_overwrites_blob_atomicity_fail_to_delete_old_twice() {
        // fail_after_operations is the highest value that makes deleting the old, overwritten block fail.
        let mut flash = common::Flash::new_with_fault(8, 60);

        // a page has 126 entries
        // the first page contains the namespace, the header for the blob_data and the first 124*32 bytes
        // the seconds page contains the blob_data header, 124*32 bytes of data and the blob_index entry
        let blob_initial = (u8::MIN..u8::MAX)
            .cycle()
            .take(124 * 32 + 124 * 32)
            .collect::<Vec<_>>();
        let blob_changed = (u8::MIN..u8::MAX)
            .rev()
            .cycle()
            .take(125 * 32 + 124 * 32)
            .collect::<Vec<_>>();
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob_initial.as_slice(),
            )
            .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob_initial
            );

            println!("{:?}", nvs.statistics().unwrap());

            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("blob"),
                blob_changed.as_slice(),
            )
            .unwrap();

            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("blob"),
                    blob_initial.as_slice()
                ),
                Err(FlashError)
            );
        }
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob_initial
        );
    }
}

// TODO overwrite small blob with fail to erase

mod defrag {
    use crate::common;
    use crate::common::Operation;
    use esp_nvs::error::Error::FlashError;
    use esp_nvs::{EntryStatistics, Key, NvsStatistics, PageStatistics};
    use pretty_assertions::assert_eq;

    #[test]
    fn defragmentation() {
        let mut flash = common::Flash::new(3);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            // overflows into second page
            // we fill all pages
            for i in 0..(125 + 126) {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("value"), i)
                    .unwrap();
            }
        }

        assert_eq!(flash.erases(), 0);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            // under the hood, the second page should be erased and reclaimed
            nvs.set(&Key::from_str("ns1"), &Key::from_str("value"), i32::MAX)
                .unwrap();

            assert_eq!(
                nvs.statistics().unwrap(),
                NvsStatistics {
                    pages: PageStatistics {
                        empty: 1,
                        active: 1,
                        full: 1,
                        erasing: 0,
                        corrupted: 0,
                    },
                    entries_per_page: vec![
                        EntryStatistics {
                            empty: 126,
                            written: 0,
                            erased: 0,
                            illegal: 0,
                        },
                        EntryStatistics {
                            empty: 0,
                            written: 0,
                            erased: 126,
                            illegal: 0,
                        },
                        EntryStatistics {
                            empty: 124,
                            written: 2,
                            erased: 0,
                            illegal: 0,
                        },
                    ],
                    entries_overall: EntryStatistics {
                        empty: 250,
                        written: 2,
                        erased: 126,
                        illegal: 0,
                    },
                }
            );
        }

        assert_eq!(flash.erases(), 1);
    }

    #[test]
    fn page_freeing_no_fault() {
        let mut flash = common::Flash::new(2);

        {
            // we fill hald the page with persistent data, the other half with erased entries
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            for i in 0u8..62 {
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("unique_{i}")),
                    i,
                )
                .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i)
                    .unwrap();
            }

            assert_eq!(
                nvs.statistics().unwrap(),
                NvsStatistics {
                    pages: PageStatistics {
                        empty: 1,
                        active: 0,
                        full: 1,
                        erasing: 0,
                        corrupted: 0,
                    },
                    entries_per_page: vec![
                        EntryStatistics {
                            empty: 0,
                            written: 64,
                            erased: 62,
                            illegal: 0,
                        },
                        EntryStatistics {
                            empty: 126,
                            written: 0,
                            erased: 0,
                            illegal: 0,
                        },
                    ],
                    entries_overall: EntryStatistics {
                        empty: 126,
                        written: 64,
                        erased: 62,
                        illegal: 0,
                    },
                }
            );
        }

        // Write another entry - this triggers defragmentation
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("trigger_defrag"),
                255u8,
            )
            .unwrap();

            // After triggering defragmentation, the old page has been erased and all valid entries
            // are now on the new page
            assert_eq!(
                nvs.statistics().unwrap(),
                NvsStatistics {
                    pages: PageStatistics {
                        empty: 1,
                        active: 1,
                        full: 0,
                        erasing: 0,
                        corrupted: 0,
                    },
                    entries_per_page: vec![
                        EntryStatistics {
                            empty: 126,
                            written: 0,
                            erased: 0,
                            illegal: 0,
                        },
                        EntryStatistics {
                            empty: 61,
                            written: 65,
                            erased: 0,
                            illegal: 0,
                        },
                    ],
                    entries_overall: EntryStatistics {
                        empty: 187,
                        written: 65,
                        erased: 0,
                        illegal: 0,
                    },
                }
            );

            // Verify data integrity - unique entries and latest duplicate value should survive
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"))
                    .unwrap(),
                255
            );
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("duplicate"))
                    .unwrap(),
                62
            );
            for i in 0u8..62 {
                assert_eq!(
                    nvs.get::<u8>(
                        &Key::from_str("ns1"),
                        &Key::from_str(&format!("unique_{i}"))
                    )
                    .unwrap(),
                    i
                );
            }
        }
    }

    #[test]
    fn page_freeing_fault_before_copy() {
        // Set up initial state with pages full and ready for defragmentation
        let mut flash = common::Flash::new_with_fault(2, 380);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            for i in 0u8..62 {
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("unique_{i}")),
                    i,
                )
                .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i)
                    .unwrap();
            }

            // Fault occurs before copying the old page to the new one
            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("trigger_defrag"),
                    255u8
                ),
                Err(FlashError)
            );
        }

        // Disable faults and verify system recovered gracefully
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 1,
                    active: 0,
                    full: 1,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 0,
                        written: 64,
                        erased: 62,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                ],
                entries_overall: EntryStatistics {
                    empty: 126,
                    written: 64,
                    erased: 62,
                    illegal: 0,
                },
            }
        );
    }

    #[test]
    fn page_freeing_fault_before_marking_as_freeing() {
        // Set up initial state with pages full and ready for defragmentation
        let mut flash = common::Flash::new_with_fault(2, 381);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            for i in 0u8..62 {
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("unique_{i}")),
                    i,
                )
                .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i)
                    .unwrap();
            }

            // Fault occurs before copying the old page to the new one
            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("trigger_defrag"),
                    255u8
                ),
                Err(FlashError)
            );
        }

        // the last successful operation was to set the state to freeing
        assert_eq!(
            flash.operations.last().unwrap(),
            &Operation::Write { offset: 0, len: 4 }
        );

        // Disable faults and verify system recovered gracefully
        flash.disable_faults();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        // All original data must be intact
        assert_eq!(
            nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("duplicate"))
                .unwrap(),
            62
        );
        for i in 0u8..62 {
            assert_eq!(
                nvs.get::<u8>(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("unique_{i}"))
                )
                .unwrap(),
                i
            );
        }

        assert_eq!(
            nvs.statistics().unwrap(),
            NvsStatistics {
                pages: PageStatistics {
                    empty: 1,
                    active: 1,
                    full: 0,
                    erasing: 0,
                    corrupted: 0,
                },
                entries_per_page: vec![
                    EntryStatistics {
                        empty: 126,
                        written: 0,
                        erased: 0,
                        illegal: 0,
                    },
                    EntryStatistics {
                        empty: 62,
                        written: 64,
                        erased: 0,
                        illegal: 0,
                    },
                ],
                entries_overall: EntryStatistics {
                    empty: 188,
                    written: 64,
                    erased: 0,
                    illegal: 0,
                },
            }
        );
    }

    #[test]
    fn page_freeing_fault_during_copy() {
        use esp_nvs::error::Error::FlashError;

        // Set up initial state with pages full and ready for defragmentation
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            for i in 0u8..62 {
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("unique_{i}")),
                    i,
                )
                .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i)
                    .unwrap();
            }
        }

        // Inject fault while copying entries to the new page during defragmentation
        // The defragmentation starts around operation 380 relative to this point.
        // Copying happens from operations 384-575 (192 operations).
        // We inject fault halfway through copying at operation 480.
        flash.fail_after_operation = flash.operations.len() + 99;

        {
            // set() will trigger defragmentation and fail during the copy phase
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("trigger_defrag"),
                255u8,
            );
            assert_eq!(result, Err(FlashError));
        }

        // the last successful operation was write an item to the new page
        assert_eq!(
            flash.operations.last().unwrap(),
            &Operation::Write {
                offset: 5152,
                len: 32
            }
        );

        // Disable faults and verify system recovers
        flash.disable_faults();
        flash.operations.clear();

        {
            let _ = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        }

        // If there are only two operations, the defragmentation was not recovered
        assert!(flash.operations.len() > 2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            // All original data must be recoverable
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("duplicate"))
                    .unwrap(),
                62
            );
            for i in 0u8..62 {
                assert_eq!(
                    nvs.get::<u8>(
                        &Key::from_str("ns1"),
                        &Key::from_str(&format!("unique_{i}"))
                    )
                    .unwrap(),
                    i
                );
            }

            assert_eq!(
                nvs.statistics().unwrap(),
                NvsStatistics {
                    pages: PageStatistics {
                        empty: 1,
                        active: 1,
                        full: 0,
                        erasing: 0,
                        corrupted: 0,
                    },
                    entries_per_page: vec![
                        EntryStatistics {
                            empty: 126,
                            written: 0,
                            erased: 0,
                            illegal: 0,
                        },
                        EntryStatistics {
                            empty: 62,
                            written: 64,
                            erased: 0,
                            illegal: 0,
                        },
                    ],
                    entries_overall: EntryStatistics {
                        empty: 188,
                        written: 64,
                        erased: 0,
                        illegal: 0,
                    },
                }
            );
        }
    }

    #[test]
    fn page_freeing_fault_after_copy_before_erase() {
        use esp_nvs::error::Error::FlashError;

        // Set up initial state
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            for i in 0u8..62 {
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str(&format!("unique_{i}")),
                    i,
                )
                .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i)
                    .unwrap();
            }
        }

        // Inject fault after all entries copied but just before erase
        // From test output: erase happens at operation #576 (196 operations after initial setup at #380)
        // Inject fault at operation 195 to fail at operation 575 (just before erase at 576)
        flash.fail_after_operation = flash.operations.len() + 195;

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str("trigger_defrag"),
                255u8,
            );
            assert_eq!(result, Err(FlashError));
        }

        // Disable faults and verify recovery
        flash.disable_faults();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            // All original data must be recoverable
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("duplicate"))
                    .unwrap(),
                62
            );
            for i in 0u8..62 {
                assert_eq!(
                    nvs.get::<u8>(
                        &Key::from_str("ns1"),
                        &Key::from_str(&format!("unique_{i}"))
                    )
                    .unwrap(),
                    i
                );
            }

            let stats = nvs.statistics().unwrap();

            // System should recover to valid state
            // After reload, the FREEING page may still exist if erase wasn't completed
            // This is correct behavior - the system preserves the intermediate state
            assert_eq!(stats.pages.corrupted, 0, "No corrupted pages");
            assert_eq!(stats.entries_overall.illegal, 0, "No illegal entries");

            // Data integrity must be preserved - all entries should be readable
            assert!(stats.entries_overall.written > 0);

            // The system may have a page in FREEING state waiting to be erased on next write
            // This is acceptable recovery behavior
        }
    }

    #[test]
    fn ensure_active_page_is_in_correct_spot_after_init() {
        // Our code depends on the invariant that the internal `Nvs::pages` vector always stores
        // the active page as the last element. This requirement was ignored when the sectors are
        // initially loaded, and this test ensures that it doesn't break again.
        //
        // Details:
        // This test overrides the same blob multiple times. All pages are allocated sequentially.
        // At some point, when overwriting the blob, the first page is defragmented, erased, and
        // marked again as active.
        // The next time the NVS is initialized, the sectors are loaded, and the pages are stored
        // sequentially in memory. So when the blob is overwritten again, the last page in
        // `Nvs::pages` is not marked as active and the defragmentation process is started again.
        // Now, when the first page is evaluated if it is eligible for defragmentation, the
        // code trips on an `unreachable!()` as the `Active` state is not expected.
        let mut flash = common::Flash::new(3);

        for i in 0..5u32 {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            let multi_page_blob: Vec<_> = (i as u8..255).cycle().take(3000).collect();
            nvs.set(
                &Key::from_str("main"),
                &Key::from_str("blob"),
                multi_page_blob.as_slice(),
            )
            .unwrap();
        }
    }

    // TODO: in case we we want to write a sized item to a page and it doesn't fit, before
    //  allocating an new empty page and defragmenting into it we can try to fill the still empty entries first
}
