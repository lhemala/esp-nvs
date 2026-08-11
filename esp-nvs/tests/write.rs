mod common;

mod key {
    use esp_nvs::Key;

    #[test]
    #[should_panic(expected = "value must be within the ascii range")]
    fn non_ascii_key() {
        let _ = Key::from_str("ungültig");
    }
}

mod set {
    use esp_nvs::error::Error;
    use esp_nvs::{
        EntryStatistics,
        ITEM_SIZE,
        Key,
        MAX_BLOB_DATA_PER_PAGE,
        MAX_BLOB_SIZE,
        Nvs,
    };
    use pretty_assertions::assert_eq;

    use crate::common;

    // TODO: test for writing namespace fails + cleanup

    /// Writes single entry items to `namespace` until the first page has `leave_free` entries left.
    ///
    /// Only meaningful on a partition that is still fresh, where everything lands on page 0 and
    /// that page is the active one.
    ///
    /// The first `set` also writes the namespace record, so page 0 goes 126 -> 124 -> 123 -> ...
    /// and `leave_free = 125` is unreachable: it trips the assert below instead of looping.
    fn fill_active_page(nvs: &mut Nvs<&mut common::Flash>, namespace: &Key, leave_free: u32) {
        for i in 0u32.. {
            let free = nvs.statistics().unwrap().entries_per_page[0].empty;
            assert!(free >= leave_free, "overshot the requested fill level");
            if free == leave_free {
                return;
            }

            nvs.set(namespace, &Key::from_str(&format!("filler{i:03}")), i as u8)
                .unwrap();
        }
    }

    /// A flash operation budget for the tests that guard against `set_blob` looping.
    ///
    /// The failure mode is an endless loop, which without a bound hangs the suite instead of
    /// failing it. Handing [`common::Flash::new_with_fault`] a budget bounds the write in flash
    /// operations rather than wall clock: it fails deterministically in milliseconds, needs no
    /// worker thread, and cannot flake on a loaded machine.
    ///
    /// The bodies below measure at ~800 operations, while a loop writes a `Full` marker, erases a
    /// sector and copies 126 entries every turn, so this budget separates the two by a wide margin.
    const SPIN_BUDGET: usize = 20_000;

    /// Asserts `result` is `expected`, naming an exhausted [`SPIN_BUDGET`] for what it is.
    ///
    /// Running out of budget surfaces as `FlashError`, which on its own says nothing about why.
    #[track_caller]
    fn assert_no_spin(result: Result<(), Error>, expected: Error) {
        assert_ne!(
            result,
            Err(Error::FlashError),
            "the write used more than {SPIN_BUDGET} flash operations instead of returning \
             {expected:?}, which means it looped rather than making progress"
        );
        assert_eq!(result, Err(expected));
    }

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

        nvs.set(&Key::from_str("hello world"), &Key::from_str("u16"), 0xAAAAu16)
            .unwrap();
        assert_eq!(
            nvs.get::<u16>(&Key::from_str("hello world"), &Key::from_str("u16"))
                .unwrap(),
            0xAAAAu16
        );
        nvs.set(&Key::from_str("hello world"), &Key::from_str("i16"), -30000i16)
            .unwrap();
        assert_eq!(
            nvs.get::<i16>(&Key::from_str("hello world"), &Key::from_str("i16"))
                .unwrap(),
            -30000i16
        );

        nvs.set(&Key::from_str("hello world"), &Key::from_str("u32"), 0xAAAAAAAAu32)
            .unwrap();
        assert_eq!(
            nvs.get::<u32>(&Key::from_str("hello world"), &Key::from_str("u32"))
                .unwrap(),
            0xAAAAAAAAu32
        );
        nvs.set(&Key::from_str("hello world"), &Key::from_str("i32"), -2000000000i32)
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
        nvs.set(&Key::from_str("hello world"), &Key::from_str("long str"), long_str)
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
            nvs.get::<Vec<u8>>(&Key::from_str("hello world"), &Key::from_str("multi page blob"))
                .unwrap(),
            multi_page_blob
        );
    }

    /// A blob version owns 128 chunk indices, one of which (0xFF) is reserved, so at most 127
    /// chunks can be addressed and deleted again.
    ///
    /// A blob that needs every one of them retires a partially filled active page first, so all of
    /// its chunks are whole `MAX_BLOB_DATA_PER_PAGE` ones. That puts the ceiling at
    /// `MAX_BLOB_SIZE - 1` no matter what the partition looked like beforehand. Anything above it
    /// would need a 128th chunk and has to be rejected rather than stored in a shape that cannot be
    /// read or deleted again.
    #[test]
    fn blob_needing_more_chunks_than_the_index_space_is_rejected() {
        let largest = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE - 1).collect::<Vec<_>>();
        let one_byte_too_long = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE).collect::<Vec<_>>();

        println!("the largest blob a fresh partition can hold is stored and survives a reopen");
        {
            // 127 chunks need 128 pages, the rest is headroom.
            let mut flash = common::Flash::new(140);

            {
                let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), largest.as_slice())
                    .unwrap();
                assert_eq!(
                    nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                        .unwrap(),
                    largest
                );
            }

            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                largest
            );
        }

        println!("one byte more is rejected and leaves the partition usable");
        // The byte guard rejects this before a single chunk is written, so nothing is left behind.
        // The partition is still sized generously so a leftover would have room to show up rather
        // than turning into a `FlashFull` that masks it.
        let mut flash = common::Flash::new(200);

        let kept = (u8::MIN..u8::MAX).rev().cycle().take(5000).collect::<Vec<_>>();
        let retry = (u8::MIN..u8::MAX).cycle().skip(3).take(200_000).collect::<Vec<_>>();

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            nvs.set(&Key::from_str("ns1"), &Key::from_str("kept"), kept.as_slice())
                .unwrap();

            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("blob"),
                    one_byte_too_long.as_slice()
                ),
                Err(Error::ValueTooLong)
            );

            // The rejected write must not have touched the value that was already stored.
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("kept"))
                    .unwrap(),
                kept
            );
        }

        println!("re-open the partition");
        // A rejected write must not leave chunks behind. They would carry the same key and the same
        // chunk indices a retry writes, so a leftover would corrupt the retried blob.
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("kept"))
                .unwrap(),
            kept
        );

        nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), retry.as_slice())
            .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            retry
        );
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("kept"))
                .unwrap(),
            kept
        );
    }

    /// The accepted blob size must not depend on how full the active page happens to be.
    ///
    /// A chunk only takes what the active page has left, so without the retire-and-restart in
    /// `set_blob` a blob that fits on a fresh partition would be rejected on one whose active page
    /// is a few entries short - even with plenty of free pages. Both layouts have to agree on the
    /// same boundary: `MAX_BLOB_SIZE - 1` in, `MAX_BLOB_SIZE` out.
    #[test]
    fn blob_size_limit_is_independent_of_the_starting_layout() {
        // 127 chunks need 128 pages, the rest is headroom for the filler page and the reserve.
        const PAGES: usize = 140;
        // Two entries left is the worst case a chunk can be handed: one for its header, one for
        // 32 bytes of data.
        const WORST_CASE_FREE_ENTRIES: u32 = 2;

        let largest = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE - 1).collect::<Vec<_>>();
        let one_byte_too_long = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE).collect::<Vec<_>>();

        for prefill in [None, Some(WORST_CASE_FREE_ENTRIES)] {
            println!("largest blob with prefill {prefill:?}");
            let mut flash = common::Flash::new(PAGES);
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();
            if let Some(leave_free) = prefill {
                fill_active_page(&mut nvs, &Key::from_str("ns1"), leave_free);
            }

            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), largest.as_slice())
                .unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                largest
            );

            println!("one byte too long with prefill {prefill:?}");
            let mut flash = common::Flash::new(PAGES);
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();
            if let Some(leave_free) = prefill {
                fill_active_page(&mut nvs, &Key::from_str("ns1"), leave_free);
            }

            assert_eq!(
                nvs.set(
                    &Key::from_str("ns1"),
                    &Key::from_str("blob"),
                    one_byte_too_long.as_slice()
                ),
                Err(Error::ValueTooLong)
            );
        }
    }

    /// A blob that no longer fits into the chunk indices left has to retire the active page and
    /// restart on a fresh one.
    ///
    /// With three entries left the first chunk could only take 64 bytes, which caps the blob at
    /// `64 + 126 * MAX_BLOB_DATA_PER_PAGE` bytes. One byte more used to be rejected on a partition
    /// that was 99% empty; it now retires that page and stores the blob in 127 whole chunks.
    #[test]
    fn blob_too_large_for_the_active_page_restarts_on_a_fresh_one() {
        const FREE_ENTRIES: u32 = 3;
        let len = (FREE_ENTRIES as usize - 1) * ITEM_SIZE + 126 * MAX_BLOB_DATA_PER_PAGE + 1;
        assert_eq!(len, 504_065);

        let blob = (u8::MIN..u8::MAX).cycle().take(len).collect::<Vec<_>>();

        let mut flash = common::Flash::new(200);
        {
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();
            fill_active_page(&mut nvs, &Key::from_str("ns1"), FREE_ENTRIES);

            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
                .unwrap();
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob
            );

            // The retired page keeps its three unused entries, the blob itself is written in whole
            // chunks that need no more than the 127 available indices.
            assert_eq!(nvs.statistics().unwrap().entries_per_page[0].empty, FREE_ENTRIES);
        }

        println!("re-open the partition");
        let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob
        );
    }

    /// Only a blob large enough to run out of chunk indices may retire the active page.
    ///
    /// An ordinary write has to keep filling the page it is handed, otherwise every write onto a
    /// nearly full page would burn the rest of that page.
    #[test]
    fn small_blob_fills_the_active_page_instead_of_skipping_it() {
        const FREE_ENTRIES: u32 = 4;

        let mut flash = common::Flash::new(4);
        let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();
        fill_active_page(&mut nvs, &Key::from_str("ns1"), FREE_ENTRIES);

        // A header entry plus two data entries, which leaves exactly one entry for the blob index.
        let blob = (u8::MIN..u8::MAX).cycle().take(2 * ITEM_SIZE).collect::<Vec<_>>();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
            .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            blob
        );

        let statistics = nvs.statistics().unwrap();
        assert_eq!(
            statistics.entries_per_page[0],
            EntryStatistics {
                empty: 0,
                written: 126,
                erased: 0,
                illegal: 0,
            }
        );
        // No page was skipped, so nothing beyond the first one has been touched.
        assert_eq!(statistics.pages.empty, 3);
    }

    /// Retiring the active page must not cost a page without buying a chunk.
    ///
    /// `get_active_page` does not only hand out fresh pages: once the reserve is down to one it
    /// goes through `defragment`, which copies a page's live entries into a new one and hands that
    /// back as the active page - partially filled, and reproducible verbatim for as long as no
    /// entry is erased. Retiring such a page unconditionally never terminates: every turn writes a
    /// `Full` marker, erases a sector and copies 126 entries, and arrives at the same state.
    ///
    /// So the retire only happens once per chunk. If the page after it still does not fit, the
    /// partial chunk is written after all, which always advances and lets the write end in the
    /// `FlashFull` (or, once the chunk indices run out, `ValueTooLong`) it belongs in.
    #[test]
    fn blob_too_large_for_the_partition_fails_instead_of_spinning() {
        println!("overwriting a blob the partition can only hold once");
        {
            // A single blob of this size fits, a second version next to it does not, so the
            // overwrite has to defragment mid-write and gets a copied page rather than a fresh one.
            let first = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE - 1).collect::<Vec<_>>();
            let second = (u8::MIN..u8::MAX)
                .rev()
                .cycle()
                .take(MAX_BLOB_SIZE - 1)
                .collect::<Vec<_>>();

            let mut flash = common::Flash::new_with_fault(130, SPIN_BUDGET);
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), first.as_slice())
                .unwrap();
            assert_no_spin(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), second.as_slice()),
                Error::FlashFull,
            );
            // The version that was already stored has to survive the failed overwrite.
            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                first
            );
        }

        println!("a blob larger than the partition, with erased entries present");
        {
            // Erased entries are what gives `defragment` something to reclaim, so they decide
            // whether it hands back a copied page at all. A partition too small for the blob has to
            // say so rather than shuffling pages forever.
            let mut flash = common::Flash::new_with_fault(120, SPIN_BUDGET);
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

            for i in 0..10u32 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("gone{i}")), i as u8)
                    .unwrap();
            }
            for i in 0..10u32 {
                nvs.delete(&Key::from_str("ns1"), &Key::from_str(&format!("gone{i}")))
                    .unwrap();
            }

            let blob = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE - 1).collect::<Vec<_>>();
            assert_no_spin(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice()),
                Error::FlashFull,
            );
        }
    }

    /// Reaches the chunk count backstop inside `set_blob`'s loop.
    ///
    /// This is the guard commit `9906399` added, and it is the one thing standing between a blob
    /// that runs out of chunk indices and permanent corruption. Reaching it needs a blob that gets
    /// past the byte guard and *still* runs out, which only happens where the retire has to give
    /// up: a partition too small to keep handing out fresh pages, so the remaining chunks come
    /// out partial and a 128th index would be needed.
    ///
    /// At this size `ValueTooLong` can only come from that in-loop guard - the byte guard rejects
    /// from `MAX_BLOB_SIZE` upwards and this blob is one byte below it - so the assertion is proof
    /// the guard was reached.
    #[test]
    fn blob_running_out_of_chunk_indices_mid_write_is_rejected() {
        let blob = (u8::MIN..u8::MAX).cycle().take(MAX_BLOB_SIZE - 1).collect::<Vec<_>>();
        let retry = (u8::MIN..u8::MAX).rev().cycle().take(5_000).collect::<Vec<_>>();

        let mut flash = common::Flash::new_with_fault(128, SPIN_BUDGET);
        {
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

            // One erased entry is what gives `defragment` something to reclaim, which is how the
            // write ends up with a partially filled page instead of a fresh one.
            nvs.set(&Key::from_str("ns1"), &Key::from_str("gone"), 1u8).unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("gone")).unwrap();

            assert_no_spin(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice()),
                Error::ValueTooLong,
            );
        }

        println!("re-open the partition");
        // Unlike the byte guard, this bail-out happens after chunks have been written, so it does
        // leave orphans behind. They carry the same key and the same chunk indices a retry writes,
        // so init has to clean them up or the retried blob is corrupted.
        flash.disable_faults();
        let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob")),
            Err(Error::KeyNotFound)
        );

        nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), retry.as_slice())
            .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            retry
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
            nvs.set(&Key::from_str("hello world"), &Key::from_str(&format!("{i}")), i)
                .unwrap();
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("hello world"), &Key::from_str(&format!("{i}")))
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
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), blob.as_slice())
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
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), blob.as_slice())
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
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), blob.as_slice())
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
            nvs.set(&Key::from_str("hello world"), &Key::from_str("val"), blob.as_slice())
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), i).unwrap();
            }
        }

        // last item on first page is unused
        assert_eq!(flash.buf[4096 - 32..4096], vec![0xffu8; 32]);

        // second page is still uninitialized
        assert_eq!(flash.buf[4096..4096 * 2], vec![0xffu8; 4096]);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns2"), &Key::from_str("another item"), u64::MIN)
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), i).unwrap();
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("item_{i}")), i)
                    .unwrap();
            }
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        let result = nvs.set::<u8>(&Key::from_str("ns1"), &Key::from_str("item_125"), 1);
        assert_eq!(result, Err(Error::FlashFull));
    }
}

mod delete {
    use esp_nvs::error::Error;
    use esp_nvs::{
        EntryStatistics,
        Key,
        NvsStatistics,
        PageStatistics,
    };
    use pretty_assertions::assert_eq;

    use crate::common;

    #[test]
    fn primitive() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("primitive"), 123)
                .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("primitive")).unwrap();

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
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
                .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("blob")).unwrap();

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
            let blob = (u8::MIN..u8::MAX).cycle().take(4096 * 2).collect::<Vec<_>>();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
                .unwrap();

            nvs.delete(&Key::from_str("ns1"), &Key::from_str("blob")).unwrap();

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
    use esp_nvs::error::Error::{
        FlashError,
        KeyNotFound,
    };
    use esp_nvs::{
        EntryStatistics,
        ITEM_SIZE,
        Key,
        MAX_BLOB_SIZE,
        NvsStatistics,
        PageStatistics,
    };
    use pretty_assertions::assert_eq;

    use crate::common;

    #[test]
    fn primitive_overwrites_primitive() {
        let mut flash = common::Flash::new(2);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("my_primitive"), 42u8)
            .unwrap();

        nvs.set(&Key::from_str("ns1"), &Key::from_str("my_primitive"), 1337u16)
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
            nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), 1u8).unwrap();

            // The fault is injected here right before the deletion of the old value
            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("item"), 2u8),
                Err(FlashError)
            );
        }

        flash.disable_faults();

        // The new value should be readable even though deletion of the old value failed
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("item")).unwrap(), 2);
    }

    #[test]
    fn blob_overwrites_blob() {
        let mut flash = common::Flash::new(6);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        let blob = (u8::MIN..u8::MAX).cycle().take(4096 * 2).collect::<Vec<_>>();

        println!("write initial value");
        nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
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
        let blob = (u8::MIN..u8::MAX).rev().cycle().take(4096 * 2).collect::<Vec<_>>();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
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
                (u8::MIN..u8::MAX).cycle().take(4096 * 2).collect::<Vec<_>>()
            } else {
                (u8::MIN..u8::MAX).rev().cycle().take(4096 * 2).collect::<Vec<_>>()
            };

            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice())
                .unwrap();
        }
    }

    /// A blob spanning three chunks is overwritten by one that fits into a single chunk.
    ///
    /// The two surplus chunks of the old blob have to be erased. If they were left behind the
    /// written entry count would be higher than the one asserted below, and re-opening the
    /// partition would make `cleanup_dirty_blobs` delete the whole blob.
    #[test]
    fn blob_shrinks_across_chunk_boundary() {
        let mut flash = common::Flash::new(6);

        // 8192 bytes need three chunks: 3968 bytes (124 of the 125 entries left on the page that
        // also holds the namespace record), 4000 bytes (125 entries of a whole page) and the
        // remaining 224 bytes.
        let big = (u8::MIN..u8::MAX).cycle().take(8192).collect::<Vec<_>>();
        // 1000 bytes fit into a single chunk.
        let small = (u8::MIN..u8::MAX).rev().cycle().take(1000).collect::<Vec<_>>();

        let statistics_after_shrink;

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            println!("write the three chunk blob");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), big.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                big
            );

            // namespace + 3 chunk headers + 8192 / 32 data entries + blob index
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
                        // namespace + first chunk (1 header + 124 data)
                        EntryStatistics {
                            empty: 0,
                            written: 1 + 125,
                            erased: 0,
                            illegal: 0,
                        },
                        // second chunk (1 header + 125 data)
                        EntryStatistics {
                            empty: 0,
                            written: 126,
                            erased: 0,
                            illegal: 0,
                        },
                        // third chunk (1 header + 7 data) + blob index
                        EntryStatistics {
                            empty: 117,
                            written: 8 + 1,
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
                        empty: 756 - 261,
                        written: 1 + 3 + 8192 / 32 + 1,
                        erased: 0,
                        illegal: 0,
                    },
                }
            );

            println!("overwrite it with the single chunk blob");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), small.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                small
            );

            // namespace + 1 chunk header + ceil(1000 / 32) data entries + blob index. Everything
            // the three chunk blob occupied is erased, including its two surplus chunks.
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
                        // namespace stays, the first old chunk is erased
                        EntryStatistics {
                            empty: 0,
                            written: 1,
                            erased: 125,
                            illegal: 0,
                        },
                        // the whole second old chunk is erased
                        EntryStatistics {
                            empty: 0,
                            written: 0,
                            erased: 126,
                            illegal: 0,
                        },
                        // new chunk (1 header + 32 data) + new blob index, old third chunk and old
                        // blob index erased
                        EntryStatistics {
                            empty: 126 - 9 - 34,
                            written: 33 + 1,
                            erased: 8 + 1,
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
                        empty: 756 - 35 - 260,
                        written: 1 + 1 + 1000_usize.div_ceil(32) as u32 + 1,
                        erased: 125 + 126 + 9,
                        illegal: 0,
                    },
                }
            );

            statistics_after_shrink = nvs.statistics().unwrap();
        }

        println!("re-open the partition");
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        // A leaked or missing chunk would let `cleanup_dirty_blobs` delete the blob during init.
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            small
        );
        assert_eq!(nvs.statistics().unwrap(), statistics_after_shrink);
    }

    /// A blob that fits into a single chunk is overwritten by one spanning three chunks.
    #[test]
    fn blob_grows_across_chunk_boundary() {
        let mut flash = common::Flash::new(6);

        let small = (u8::MIN..u8::MAX).cycle().take(1000).collect::<Vec<_>>();
        let big = (u8::MIN..u8::MAX).rev().cycle().take(8192).collect::<Vec<_>>();

        let statistics_after_growth;

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            println!("write the single chunk blob");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), small.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                small
            );

            // namespace + 1 chunk header + ceil(1000 / 32) data entries + blob index
            assert_eq!(
                nvs.statistics().unwrap(),
                NvsStatistics {
                    pages: PageStatistics {
                        empty: 5,
                        active: 1,
                        full: 0,
                        erasing: 0,
                        corrupted: 0,
                    },
                    entries_per_page: vec![
                        EntryStatistics {
                            empty: 126 - 35,
                            written: 1 + 33 + 1,
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
                        empty: 756 - 35,
                        written: 1 + 1 + 1000_usize.div_ceil(32) as u32 + 1,
                        erased: 0,
                        illegal: 0,
                    },
                }
            );

            println!("overwrite it with the three chunk blob");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), big.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                big
            );

            // The 91 entries left on the first page take 2880 bytes, the second page 4000 bytes
            // and the remaining 1312 bytes end up on the third page: namespace + 3 chunk headers +
            // 8192 / 32 data entries + blob index.
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
                        // namespace + first new chunk (1 header + 90 data), old chunk and old blob
                        // index erased
                        EntryStatistics {
                            empty: 0,
                            written: 1 + 91,
                            erased: 33 + 1,
                            illegal: 0,
                        },
                        // second new chunk (1 header + 125 data)
                        EntryStatistics {
                            empty: 0,
                            written: 126,
                            erased: 0,
                            illegal: 0,
                        },
                        // third new chunk (1 header + 41 data) + new blob index
                        EntryStatistics {
                            empty: 126 - 43,
                            written: 42 + 1,
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
                        empty: 756 - 261 - 34,
                        written: 1 + 3 + 8192 / 32 + 1,
                        erased: 34,
                        illegal: 0,
                    },
                }
            );

            statistics_after_growth = nvs.statistics().unwrap();
        }

        println!("re-open the partition");
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            big
        );
        assert_eq!(nvs.statistics().unwrap(), statistics_after_growth);
    }

    /// Shrinking and growing a blob while the chunk count stays the same.
    #[test]
    fn blob_resizes_within_the_same_chunk_count() {
        let mut flash = common::Flash::new(6);

        // 5000 bytes span two chunks: 3968 bytes next to the namespace record and 1032 bytes.
        let large = (u8::MIN..u8::MAX).cycle().take(5000).collect::<Vec<_>>();
        // 4500 bytes still span two chunks, the split just moves.
        let smaller = (u8::MIN..u8::MAX).rev().cycle().take(4500).collect::<Vec<_>>();
        // 5000 bytes again, with yet another pattern so stale chunks show up as wrong content.
        let large_again = (u8::MIN..u8::MAX).cycle().skip(7).take(5000).collect::<Vec<_>>();

        let statistics_after_regrowth;

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            println!("write 5000 bytes");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), large.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                large
            );

            let statistics = nvs.statistics().unwrap();
            // namespace + 2 chunk headers + 124 + ceil(1032 / 32) data entries + blob index
            assert_eq!(
                statistics.entries_overall,
                EntryStatistics {
                    empty: 756 - 161,
                    written: 1 + 2 + 124 + 1032_usize.div_ceil(32) as u32 + 1,
                    erased: 0,
                    illegal: 0,
                }
            );

            println!("shrink it to 4500 bytes");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), smaller.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                smaller
            );

            let statistics = nvs.statistics().unwrap();
            // 2880 bytes fill the second page, 1620 bytes go to the third one: namespace +
            // 2 chunk headers + 90 + ceil(1620 / 32) data entries + blob index
            assert_eq!(
                statistics.entries_overall,
                EntryStatistics {
                    empty: 756 - 145 - 160,
                    written: 1 + 2 + 90 + 1620_usize.div_ceil(32) as u32 + 1,
                    erased: 125 + 35,
                    illegal: 0,
                }
            );

            println!("grow it back to 5000 bytes");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), large_again.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                large_again
            );

            let statistics = nvs.statistics().unwrap();
            // 2304 bytes fill the third page, 2696 bytes go to the fourth one: namespace +
            // 2 chunk headers + 72 + ceil(2696 / 32) data entries + blob index
            assert_eq!(
                statistics.entries_overall,
                EntryStatistics {
                    empty: 756 - 161 - 304,
                    written: 1 + 2 + 72 + 2696_usize.div_ceil(32) as u32 + 1,
                    erased: 125 + 126 + 53,
                    illegal: 0,
                }
            );

            statistics_after_regrowth = statistics.entries_overall;
        }

        println!("re-open the partition");
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            large_again
        );
        assert_eq!(nvs.statistics().unwrap().entries_overall, statistics_after_regrowth);
    }

    /// Pins the range `delete_blob_data` sweeps.
    ///
    /// This blob uses all 127 chunk indices a blob version owns, which is legal, so the test does
    /// not exercise the chunk count guard in `set_blob`. What it does exercise is the cleanup of
    /// the old version: the last index it has to reach is `chunk_start + 126`, so narrowing the
    /// sweep by a single chunk leaves the last one behind and shows up as a higher written count
    /// below.
    #[test]
    fn largest_blob_overwrites_itself() {
        // Both versions coexist until the old one is deleted, so the partition has to hold two of
        // them (128 pages each) plus a page to reclaim into.
        let mut flash = common::Flash::new(260);

        let before = (u8::MIN..u8::MAX)
            .cycle()
            .take(MAX_BLOB_SIZE - ITEM_SIZE)
            .collect::<Vec<_>>();
        let after = (u8::MIN..u8::MAX)
            .rev()
            .cycle()
            .take(MAX_BLOB_SIZE - ITEM_SIZE)
            .collect::<Vec<_>>();

        // namespace + 127 chunk headers + (MAX_BLOB_SIZE - ITEM_SIZE) / 32 data entries + index
        let live_entries = 1 + 127 + (MAX_BLOB_SIZE - ITEM_SIZE) as u32 / 32 + 1;

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            println!("write the 127 chunk blob");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), before.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                before
            );
            assert_eq!(nvs.statistics().unwrap().entries_overall.written, live_entries);

            println!("overwrite it with another 127 chunk blob");
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), after.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                after
            );
            // Everything of the old version except the namespace record is gone; a chunk index
            // that cannot be addressed by `delete_blob_data` would show up as a higher written
            // count here.
            assert_eq!(
                nvs.statistics().unwrap().entries_overall,
                EntryStatistics {
                    empty: 260 * 126 - live_entries - (live_entries - 1),
                    written: live_entries,
                    erased: live_entries - 1,
                    illegal: 0,
                }
            );
        }

        println!("re-open the partition");
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            after
        );
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice()),
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
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_initial.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob_initial
            );

            let blob_changed = (u8::MIN..u8::MAX).rev().cycle().take(4096).collect::<Vec<_>>();

            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_changed.as_slice()),
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
        // fail_after_operations is the highest value that makes deleting the old, overwritten block
        // fail.
        let mut flash = common::Flash::new_with_fault(5, 39);

        // a page has 126 entries
        // the first page contains the namespace, the header for the blob_data and the first 124*32
        // bytes the seconds page contains the blob_data header, 124*32 bytes of data and
        // the blob_index entry
        let blob_initial = (u8::MIN..u8::MAX).cycle().take(124 * 32 + 124 * 32).collect::<Vec<_>>();
        let blob_changed = (u8::MIN..u8::MAX)
            .rev()
            .cycle()
            .take(125 * 32 + 124 * 32)
            .collect::<Vec<_>>();
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_initial.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob_initial
            );

            println!("{:?}", nvs.statistics().unwrap());

            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_changed.as_slice()),
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
        // fail_after_operations is the highest value that makes deleting the old, overwritten block
        // fail.
        let mut flash = common::Flash::new_with_fault(8, 60);

        // a page has 126 entries
        // the first page contains the namespace, the header for the blob_data and the first 124*32
        // bytes the seconds page contains the blob_data header, 124*32 bytes of data and
        // the blob_index entry
        let blob_initial = (u8::MIN..u8::MAX).cycle().take(124 * 32 + 124 * 32).collect::<Vec<_>>();
        let blob_changed = (u8::MIN..u8::MAX)
            .rev()
            .cycle()
            .take(125 * 32 + 124 * 32)
            .collect::<Vec<_>>();
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_initial.as_slice())
                .unwrap();

            assert_eq!(
                nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                    .unwrap(),
                blob_initial
            );

            println!("{:?}", nvs.statistics().unwrap());

            nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_changed.as_slice())
                .unwrap();

            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob_initial.as_slice()),
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
    use esp_nvs::error::Error::FlashError;
    use esp_nvs::{
        EntryStatistics,
        Key,
        NvsStatistics,
        PageStatistics,
    };
    use pretty_assertions::assert_eq;

    use crate::common;
    use crate::common::Operation;

    #[test]
    fn defragmentation() {
        let mut flash = common::Flash::new(3);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            // overflows into second page
            // we fill all pages
            for i in 0..(125 + 126) {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("value"), i).unwrap();
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")), i)
                    .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i).unwrap();
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
            nvs.set(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"), 255u8)
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
                    nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")))
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")), i)
                    .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i).unwrap();
            }

            // Fault occurs before copying the old page to the new one
            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"), 255u8),
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")), i)
                    .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i).unwrap();
            }

            // Fault occurs before copying the old page to the new one
            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"), 255u8),
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
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")))
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")), i)
                    .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i).unwrap();
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
            let result = nvs.set(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"), 255u8);
            assert_eq!(result, Err(FlashError));
        }

        // the last successful operation was write an item to the new page
        assert_eq!(
            flash.operations.last().unwrap(),
            &Operation::Write { offset: 5152, len: 32 }
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
                    nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")))
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
                nvs.set(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")), i)
                    .unwrap();
            }
            for i in 0u8..63 {
                nvs.set(&Key::from_str("ns1"), &Key::from_str("duplicate"), i).unwrap();
            }
        }

        // Inject fault after all entries copied but just before erase
        // From test output: erase happens at operation #576 (196 operations after initial setup at
        // #380) Inject fault at operation 195 to fail at operation 575 (just before erase
        // at 576)
        flash.fail_after_operation = flash.operations.len() + 195;

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.set(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"), 255u8);
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
                    nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str(&format!("unique_{i}")))
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
    //  allocating an new empty page and defragmenting into it we can try to fill the still empty
    // entries first
}

mod purge {
    use esp_nvs::error::Error::KeyNotFound;
    use esp_nvs::{
        ITEM_SIZE,
        Key,
    };
    use pretty_assertions::assert_eq;

    use crate::common;

    fn entry_range(index: usize) -> core::ops::Range<usize> {
        let start = common::ITEM_OFFSET + index * ITEM_SIZE;
        start..start + ITEM_SIZE
    }

    fn span_range(first: usize, last: usize) -> core::ops::Range<usize> {
        entry_range(first).start..entry_range(last).end
    }

    #[test]
    fn continuous_purge_zeroes_deleted_primitive() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set_purge_mode(true);
            nvs.set(&Key::from_str("ns1"), &Key::from_str("secret"), 0xAABBCCDDu32)
                .unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("secret")).unwrap();
        }

        // The value entry is physically zeroed.
        assert_eq!(flash.buf[entry_range(1)], vec![0u8; ITEM_SIZE]);

        // Reloading the purged flash succeeds and the key is gone.
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<u32>(&Key::from_str("ns1"), &Key::from_str("secret"))
                .err()
                .unwrap(),
            KeyNotFound
        );
    }

    #[test]
    fn continuous_purge_zeroes_overwritten_primitive() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set_purge_mode(true);
            nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 42u8).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 99u8).unwrap();

            assert_eq!(nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k")).unwrap(), 99);
        }

        // The overwritten value is zeroed, the new value remains.
        assert_eq!(flash.buf[entry_range(1)], vec![0u8; ITEM_SIZE]);
        assert_ne!(flash.buf[entry_range(2)], vec![0u8; ITEM_SIZE]);
    }

    #[test]
    fn continuous_purge_zeroes_deleted_string() {
        let mut flash = common::Flash::new(2);

        // 40 bytes + null terminator spans two data entries, so the item occupies entries 1..=3.
        let value = "A".repeat(40);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set_purge_mode(true);
            nvs.set(&Key::from_str("ns1"), &Key::from_str("s"), value.as_str())
                .unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("s")).unwrap();
        }

        // Header and both data entries are zeroed.
        assert_eq!(flash.buf[span_range(1, 3)], vec![0u8; 3 * ITEM_SIZE]);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("s"))
                .err()
                .unwrap(),
            KeyNotFound
        );
    }

    #[test]
    fn continuous_purge_zeroes_deleted_blob() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set_purge_mode(true);
            // A 3-byte blob: chunk header + data at entries 1..=2, blob index at entry 3.
            nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), [1u8, 2, 3].as_slice())
                .unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("b")).unwrap();
        }

        // Chunk (entries 1 and 2) and blob index (entry 3) are all zeroed.
        assert_eq!(flash.buf[span_range(1, 3)], vec![0u8; 3 * ITEM_SIZE]);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b"))
                .err()
                .unwrap(),
            KeyNotFound
        );
    }

    #[test]
    fn default_mode_leaves_deleted_value_in_flash() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            assert!(!nvs.purge_mode());
            nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 0x11223344u32)
                .unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("k")).unwrap();
        }

        // Without purge mode, the deleted value's bytes remain physically present.
        assert_ne!(flash.buf[entry_range(1)], vec![0u8; ITEM_SIZE]);
    }

    #[test]
    fn one_time_purge_all_zeroes_existing_erased_data() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 0xDEADBEEFu32)
                .unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("k")).unwrap();

            nvs.purge_all(&Key::from_str("ns1")).unwrap();
        }

        assert_eq!(flash.buf[entry_range(1)], vec![0u8; ITEM_SIZE]);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<u32>(&Key::from_str("ns1"), &Key::from_str("k"))
                .err()
                .unwrap(),
            KeyNotFound
        );
    }

    #[test]
    fn purge_all_only_affects_target_namespace() {
        let mut flash = common::Flash::new(2);

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns_a"), &Key::from_str("k"), 0xAAu8).unwrap();
            nvs.set(&Key::from_str("ns_b"), &Key::from_str("k"), 0xBBu8).unwrap();
            nvs.delete(&Key::from_str("ns_a"), &Key::from_str("k")).unwrap();
            nvs.delete(&Key::from_str("ns_b"), &Key::from_str("k")).unwrap();

            nvs.purge_all(&Key::from_str("ns_a")).unwrap();
        }

        // Only `ns_a` erased value is scrubbed, `ns_b` erased value remains untouched.
        assert_eq!(flash.buf[entry_range(1)], vec![0u8; ITEM_SIZE]);
        assert_ne!(flash.buf[entry_range(3)], vec![0u8; ITEM_SIZE]);

        // Reloading the partition still succeeds.
        esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
    }

    #[test]
    fn purge_all_scrubs_overwritten_value_keeping_live_keys() {
        let mut flash = common::Flash::new(2);

        let namespace = Key::from_str("ns1");
        let key1 = Key::from_str("k1");
        let key2 = Key::from_str("k2");

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&namespace, &key1, 0x11111111u32).unwrap();
            nvs.set(&namespace, &key2, 0x33333333u32).unwrap();
            nvs.set(&namespace, &key1, 0x22222222u32).unwrap();

            nvs.purge_all(&namespace).unwrap();

            // Both live values are unaffected by the purge.
            assert_eq!(nvs.get::<u32>(&namespace, &key1).unwrap(), 0x22222222);
            assert_eq!(nvs.get::<u32>(&namespace, &key2).unwrap(), 0x33333333);
        }

        // Only the stale `k1` value is physically zeroed, both live entries remain.
        assert_eq!(flash.buf[entry_range(1)], vec![0u8; ITEM_SIZE]);
        assert_ne!(flash.buf[entry_range(2)], vec![0u8; ITEM_SIZE]);
        assert_ne!(flash.buf[entry_range(3)], vec![0u8; ITEM_SIZE]);

        // Reloading confirms both keys survive with their expected values.
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(nvs.get::<u32>(&namespace, &key1).unwrap(), 0x22222222);
        assert_eq!(nvs.get::<u32>(&namespace, &key2).unwrap(), 0x33333333);
    }

    #[test]
    fn purge_all_missing_namespace_is_noop() {
        let mut flash = common::Flash::new(2);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 1u8).unwrap();

        nvs.purge_all(&Key::from_str("absent")).unwrap();

        // Unrelated data is still readable.
        assert_eq!(nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k")).unwrap(), 1);
    }
}
