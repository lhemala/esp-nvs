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
        ENTRIES_PER_PAGE,
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

    /// `PageFull` is an internal "try another page" signal, documented as such on the error type. A
    /// caller must never see it: once there is no other page to try, the honest answer is that the
    /// partition is out of room.
    ///
    /// It escaped when the retry page could not hold the value either, which is reachable because
    /// the page after a retire is not guaranteed to be a fresh one - with the reserve down to a
    /// single free page, `get_active_page` goes through defragmentation and hands back a partially
    /// filled copy.
    #[test]
    fn a_string_that_no_longer_fits_reports_flash_full_not_page_full() {
        let mut flash = common::Flash::new(3);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        // Ten entry strings, so the partition runs out part way through a page rather than exactly
        // at a boundary.
        let value = "x".repeat(9 * ITEM_SIZE);
        let mut written = 0;
        let mut outcome = None;
        for i in 0u32..1000 {
            match nvs.set(
                &Key::from_str("ns1"),
                &Key::from_str(&format!("s{i:03}")),
                value.as_str(),
            ) {
                Ok(()) => written += 1,
                Err(e) => {
                    outcome = Some(e);
                    break;
                }
            }
        }

        assert!(
            written > 0,
            "nothing was stored, the partition is too small to be meaningful"
        );
        assert_eq!(outcome, Some(Error::FlashFull), "an internal signal reached the caller");

        // The failed write leaves the instance intact rather than dropping the page it was holding.
        assert_eq!(nvs.keys().count(), written);
        for i in 0..written {
            assert_eq!(
                nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str(&format!("s{i:03}")))
                    .unwrap(),
                value
            );
        }
    }

    /// Creating a namespace can fill the page it lands on, which sends the write to the next one.
    /// The page left behind still holds live entries and has been popped out of the page list, so
    /// it has to be put back: dropping it took every key on it out of the running instance and
    /// leaked the sector, since neither the page list nor the reserve knows about it any more.
    #[test]
    fn a_namespace_that_fills_its_page_does_not_drop_it() {
        let mut flash = common::Flash::new(4);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        // One entry left on page 0, which the new namespace's record takes.
        fill_active_page(&mut nvs, &Key::from_str("ns1"), 1);
        let filled = nvs.keys().count();
        let pages = nvs.statistics().unwrap().entries_per_page.len();

        nvs.set(&Key::from_str("ns2"), &Key::from_str("k"), 42u8).unwrap();

        assert_eq!(
            nvs.statistics().unwrap().entries_per_page.len(),
            pages,
            "a page went missing from the instance"
        );
        assert_eq!(nvs.keys().count(), filled + 1, "the filled page's keys are gone");
        assert_eq!(nvs.get::<u8>(&Key::from_str("ns2"), &Key::from_str("k")).unwrap(), 42);
        assert_eq!(
            nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("filler000"))
                .unwrap(),
            0
        );
    }

    /// A string is stored as one item on one page, with a null terminator appended, so the longest
    /// one that fits is `MAX_BLOB_DATA_PER_PAGE - 1` bytes. Unlike a blob it is never split into
    /// chunks, so there is no growing the partition out of this: one byte more is rejected however
    /// much room is free.
    #[test]
    fn string_at_the_length_limit_is_accepted_and_one_past_it_is_rejected() {
        // Plenty of free pages, so a rejection cannot be confused with running out of room.
        let mut flash = common::Flash::new(8);

        let longest = "x".repeat(MAX_BLOB_DATA_PER_PAGE - 1);
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

            nvs.set(&Key::from_str("ns1"), &Key::from_str("s"), longest.as_str())
                .unwrap();
            assert_eq!(
                nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("s")).unwrap(),
                longest
            );

            // One byte more, with the same partition and the same free space.
            let too_long = "x".repeat(MAX_BLOB_DATA_PER_PAGE);
            assert_eq!(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("t"), too_long.as_str()),
                Err(Error::ValueTooLong)
            );
            // The rejected write leaves nothing behind and the accepted one is untouched.
            assert_eq!(
                nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("t")),
                Err(Error::KeyNotFound)
            );
        }

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("s")).unwrap(),
            longest
        );
        // The longest string fills a page exactly, header plus every data entry, which means it
        // cannot share one with the namespace record: page 0 keeps just that record and the string
        // goes to the next page.
        let per_page = nvs.statistics().unwrap().entries_per_page;
        assert_eq!(per_page[0].written, 1);
        assert_eq!(per_page[1].written, ENTRIES_PER_PAGE as u32);
    }

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

    /// Fills `flash` with single entry items until nothing more fits, then erases the ones at
    /// `erase_at` again, and arms a [`SPIN_BUDGET`] for whatever the caller does next. Returns how
    /// many items were written.
    ///
    /// This is the layout `defragment` has to work with when the reserve is down to its last free
    /// page: the number of erased entries on the page it reclaims is exactly the number of free
    /// entries the copy it hands back as the new active page will have.
    ///
    /// The fill gets a budget of its own and the caller opens the partition again afterwards,
    /// because filling a large partition costs far more flash operations than the write under test
    /// is allowed to, and a budget that covered both would no longer separate a write from a spin.
    /// The fill's own budget is generous - it measures at under 300 operations per page - but
    /// finite on purpose, so a spin in the fill fails the test rather than hanging the suite.
    fn fill_partition(
        flash: &mut common::Flash,
        namespace: &Key,
        pages: usize,
        erase_at: impl Fn(usize) -> Vec<usize>,
    ) -> usize {
        let mut written = 0;
        flash.arm_fault(pages * SPIN_BUDGET);
        {
            let mut nvs = Nvs::new(0, flash.len(), &mut *flash).unwrap();
            for i in 0..pages * ENTRIES_PER_PAGE {
                if nvs
                    .set(namespace, &Key::from_str(&format!("k{i:05}")), (i % 251) as u8)
                    .is_err()
                {
                    break;
                }
                written += 1;
            }
            for i in erase_at(written) {
                nvs.delete(namespace, &Key::from_str(&format!("k{i:05}"))).unwrap();
            }
        }
        flash.arm_fault(SPIN_BUDGET);
        written
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

        // Zero and one free entry are the layouts where the active page cannot hold a chunk at all,
        // so they exercise the other retire in `set_blob` rather than the chunk index one. Neither
        // reaches its `FlashFull` bail-out - with 140 pages the reserve is nowhere near exhausted,
        // so `Some(1)` retires once with the budget unspent and `Some(0)` finds no active page to
        // retire at all. They are here because the boundary has to come out the same whichever
        // retire the layout happens to hit, not as coverage of the bail-out; the tests below own
        // that.
        for prefill in [None, Some(0), Some(1), Some(WORST_CASE_FREE_ENTRIES)] {
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

    /// A full partition with exactly one erased entry must report `FlashFull`, not spin.
    ///
    /// Once the reserve is down to one free page, `get_active_page` goes through `defragment`,
    /// which copies the live entries of the page it reclaims into that reserve page and pushes the
    /// erased source back into the reserve. The free page count is therefore invariant, and the
    /// copy it hands back as the new active page has exactly as many free entries as the source had
    /// erased ones. With exactly one erased entry that is one free entry - one short of the two a
    /// blob chunk needs for its header and its first data entry - so the retire that follows
    /// arrives at a state indistinguishable from the one before it, and did so forever: every turn
    /// wrote a `Full` marker, erased a sector and copied 125 entries.
    ///
    /// This hung on v0.5.0, and a hang is the harmless reading of it. At roughly a thousand erases
    /// per second it wears a sector past a typical 100k cycle NOR endurance in under a minute, so
    /// the device does not recover by being power cycled.
    ///
    /// `FlashFull` is the right answer here rather than `ValueTooLong`: the blob is not too long
    /// for anything. The condition is `free_entries <= 1` and never looks at the length, so a one
    /// byte blob spins exactly like the largest one - it still needs a header entry and a data
    /// entry.
    #[test]
    fn blob_on_a_full_partition_with_one_erased_entry_fails_instead_of_spinning() {
        let mut flash = common::Flash::new(3);
        let written = fill_partition(&mut flash, &Key::from_str("ns1"), 3, |_| vec![0]);
        assert_eq!(written, 251, "the fill no longer fills the partition");
        {
            let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();
            // `entries_per_page` has one entry per page the instance still tracks, in the page list
            // plus the reserve, so a dropped page shows up as a missing one.
            let pages_before = nvs.statistics().unwrap().entries_per_page.len();
            let keys_before = nvs.keys().count();

            assert_no_spin(
                nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), [0u8; 64].as_slice()),
                Error::FlashFull,
            );

            // `FlashFull` is an error the caller handles and carries on from, so the *same*
            // instance has to come out of it intact. `get_active_page` pops the active page out of
            // the page list, and a bail-out that forgot to hand it back dropped every live entry on
            // it and leaked its sector out of both the page list and the reserve - invisible to a
            // test that only looks after `Nvs::new` has rebuilt everything from flash.
            assert_eq!(nvs.statistics().unwrap().entries_per_page.len(), pages_before);
            assert_eq!(nvs.keys().count(), keys_before);
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k00001")).unwrap(),
                1
            );
            assert_eq!(
                nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k00250")).unwrap(),
                250
            );
        }

        println!("re-open the partition");
        // The bail-out happens after `mark_as_full` markers have been written, so the partition it
        // leaves behind has to still be readable and writable.
        flash.disable_faults();
        let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob")),
            Err(Error::KeyNotFound)
        );
        assert_eq!(
            nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k00001")).unwrap(),
            1
        );
        assert_eq!(
            nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k00250")).unwrap(),
            250
        );

        // Freeing enough entries has to make the very same write succeed again.
        for i in 1..4u32 {
            nvs.delete(&Key::from_str("ns1"), &Key::from_str(&format!("k{i:05}")))
                .unwrap();
        }
        nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), [0u8; 64].as_slice())
            .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                .unwrap(),
            vec![0u8; 64]
        );
    }

    /// The number of erased entries decides whether the write can proceed, and every count has to
    /// terminate.
    ///
    /// It is a sharp signature: the copy `defragment` hands back has one free entry per erased
    /// entry on the page it reclaimed, and only *exactly one* leaves it one short of a chunk while
    /// still handing back a page at all. Zero and two or more always terminated; one is the trigger
    /// and is covered explicitly.
    ///
    /// Four is the first count that fits a 64 byte blob - a chunk header, two data entries and the
    /// blob index - and it has to keep succeeding, which is what stops the guard from being an
    /// unconditional `FlashFull`.
    #[test]
    fn blob_terminates_for_every_erased_entry_count() {
        for pages in [3usize, 8] {
            for erased in 0..=4usize {
                for size in [64usize, MAX_BLOB_SIZE - 1] {
                    println!("pages={pages} erased={erased} size={size}");

                    let mut flash = common::Flash::new(pages);
                    fill_partition(&mut flash, &Key::from_str("ns1"), pages, |_| (0..erased).collect());
                    let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

                    let blob = (u8::MIN..u8::MAX).cycle().take(size).collect::<Vec<_>>();
                    let result = nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice());

                    if erased == 4 && size == 64 {
                        assert_eq!(result, Ok(()));
                        assert_eq!(
                            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("blob"))
                                .unwrap(),
                            blob
                        );
                    } else {
                        assert_no_spin(result, Error::FlashFull);
                    }
                }
            }
        }
    }

    /// The spin was not tied to a partition size, a blob size or where the erased entry sat.
    ///
    /// Every combination below hung before the guard: the loop never looks at any of them, it only
    /// ever sees an active page it cannot use. The spread is here so a future change that makes the
    /// guard depend on one of them fails rather than passing on the minimal case alone.
    #[test]
    fn blob_terminates_across_partition_and_blob_sizes() {
        // 4001 straddles `MAX_BLOB_DATA_PER_PAGE`, so it is the smallest blob that needs a second
        // chunk; 504_065 is the size that commit `4b2db32` made storable; 507_999 is the ceiling.
        let sizes: &[usize] = &[64, 4000, 4001, 100_000, 504_065, MAX_BLOB_SIZE - 1];

        for (pages, sizes) in [
            (3usize, sizes),
            (8, &sizes[..4]),
            (20, &sizes[..2]),
            (40, &sizes[..2]),
            (128, &sizes[5..]),
            (130, &sizes[..1]),
        ] {
            for size in sizes {
                println!("pages={pages} size={size}");

                let mut flash = common::Flash::new(pages);
                fill_partition(&mut flash, &Key::from_str("ns1"), pages, |_| vec![0]);
                let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

                let blob = (u8::MIN..u8::MAX).cycle().take(*size).collect::<Vec<_>>();
                assert_no_spin(
                    nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), blob.as_slice()),
                    Error::FlashFull,
                );
            }
        }

        println!("the erased entry's position does not matter either");
        for pages in [3usize, 8] {
            for pick in [0usize, 1, 2] {
                let mut flash = common::Flash::new(pages);
                // First, middle and last of what was written, which puts the erased entry on the
                // first page, somewhere in the middle and on the last one.
                fill_partition(&mut flash, &Key::from_str("ns1"), pages, |written| {
                    vec![[0, written / 2, written - 1][pick]]
                });
                let mut nvs = Nvs::new(0, flash.len(), &mut flash).unwrap();

                assert_no_spin(
                    nvs.set(&Key::from_str("ns1"), &Key::from_str("blob"), [0u8; 64].as_slice()),
                    Error::FlashFull,
                );
            }
        }
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

    /// The type a key held before says nothing about what may be written to it next, so every
    /// combination has to end up with the new value under the key and the old item gone.
    ///
    /// Writing a blob over a key holding a string or a primitive used to return `Ok(())` and then
    /// be unreadable: the old item was only deleted when the key had held a blob before, and
    /// `load_item` returns the older of two items with the same key, so the leftover shadowed the
    /// blob that had just been written. Writing a primitive over a longer item erased one entry
    /// instead of the item's whole span, which left the tail of a string behind and orphaned a
    /// blob's chunks.
    #[test]
    fn set_replaces_a_value_of_any_previous_type() {
        // A blob long enough to span several chunks, so a stale index leaves chunks behind too.
        let multi_chunk: Vec<u8> = (0u8..=255).cycle().take(9000).collect();

        for from in Value::ALL {
            for to in Value::ALL {
                println!("{from:?} -> {to:?}");

                let mut flash = common::Flash::new(8);
                {
                    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                    from.write(&mut nvs, &multi_chunk);
                    to.write(&mut nvs, &multi_chunk);

                    to.assert_readable(&mut nvs, &multi_chunk, "before reopen");
                    assert_eq!(
                        nvs.keys().count(),
                        1,
                        "{from:?} -> {to:?}: the replaced item is still on flash"
                    );
                }

                let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                to.assert_readable(&mut nvs, &multi_chunk, "after reopen");
                assert_eq!(
                    nvs.keys().count(),
                    1,
                    "{from:?} -> {to:?}: the replaced item survived a reopen"
                );
            }
        }
    }

    /// Replacing a multi-chunk blob with a primitive has to erase the chunks, not just the index.
    /// Counting entries is what catches an orphan: the key reads back correctly either way, because
    /// a stale chunk is only reachable through the index that is now gone.
    ///
    /// This is the direction that used to leak. `set_primitive` erased a single entry and knew
    /// nothing about chunks, so replacing a three chunk blob left its 284 entries written forever.
    #[test]
    fn multi_chunk_blob_replaced_by_a_primitive_erases_its_chunks() {
        let mut flash = common::Flash::new(8);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        let blob: Vec<u8> = (0u8..=255).cycle().take(9000).collect();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), blob.as_slice())
            .unwrap();

        // The blob splits into three chunks, each rounding its data up to whole entries, plus the
        // namespace record and the blob index.
        let before = nvs.statistics().unwrap().entries_overall;
        assert_eq!(before.written, 287);
        assert_eq!(before.erased, 0);

        nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 42u8).unwrap();

        // Only the namespace and the primitive are left written, everything else is erased.
        let after = nvs.statistics().unwrap().entries_overall;
        assert_eq!(after.written, 1 + 1, "the old chunks were not erased");
        assert_eq!(after.erased, before.written - 1);

        assert_eq!(nvs.get::<u8>(&Key::from_str("ns1"), &Key::from_str("k")).unwrap(), 42);
    }

    /// The same for a string, which spans several entries but has no chunks. `set_primitive` used
    /// to erase one entry regardless of the item's span, leaving the string's data behind.
    #[test]
    fn string_replaced_by_a_primitive_erases_its_whole_span() {
        let mut flash = common::Flash::new(4);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();

        // Long enough to need several data entries on top of its header.
        let long = "x".repeat(200);
        nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), long.as_str())
            .unwrap();

        let before = nvs.statistics().unwrap().entries_overall;
        assert_eq!(before.written, 1 + 1 + 201_u32.div_ceil(ITEM_SIZE as u32));

        nvs.set(&Key::from_str("ns1"), &Key::from_str("k"), 42u8).unwrap();

        let after = nvs.statistics().unwrap().entries_overall;
        assert_eq!(after.written, 1 + 1, "the string's data entries were not erased");
        assert_eq!(after.erased, before.written - 1);
    }

    /// A fault while replacing a value of a different type must leave exactly one readable value.
    /// The new value is written before the old one is erased, so whichever survives, the key is
    /// never left holding neither.
    #[test]
    fn faulted_replacement_of_a_different_type_leaves_one_value() {
        let multi_chunk: Vec<u8> = (0u8..=255).cycle().take(9000).collect();

        for from in Value::ALL {
            for to in Value::ALL {
                // Sweep the fault across the whole sequence rather than guessing where the
                // interesting point is. Faults landing in the first write are skipped: the key
                // never held the old value, so there is nothing to preserve.
                let mut checked = 0;
                for fail_after in 0..700 {
                    let mut flash = common::Flash::new_with_fault(8, fail_after);
                    {
                        let mut nvs = match esp_nvs::Nvs::new(0, flash.len(), &mut flash) {
                            Ok(nvs) => nvs,
                            Err(_) => continue,
                        };
                        if from.try_write(&mut nvs, &multi_chunk).is_err() {
                            continue;
                        }
                        // Either outcome is fine, the faulted write may or may not have got there.
                        let _ = to.try_write(&mut nvs, &multi_chunk);
                    }
                    flash.disable_faults();

                    let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                    let old_ok = from.is_readable(&mut nvs, &multi_chunk);
                    let new_ok = to.is_readable(&mut nvs, &multi_chunk);
                    assert!(
                        old_ok || new_ok,
                        "{from:?} -> {to:?} faulted after {fail_after} operations: \
                         the key holds neither value"
                    );
                    checked += 1;
                }
                assert!(checked > 0, "{from:?} -> {to:?}: no fault landed after the first write");
            }
        }
    }

    /// The four shapes a value can take on flash: a primitive, a string, a blob short enough for a
    /// single chunk, and one that needs several.
    #[derive(Clone, Copy, Debug)]
    enum Value {
        Primitive,
        Str,
        SingleChunkBlob,
        MultiChunkBlob,
    }

    impl Value {
        const ALL: [Value; 4] = [
            Value::Primitive,
            Value::Str,
            Value::SingleChunkBlob,
            Value::MultiChunkBlob,
        ];

        fn try_write(
            self,
            nvs: &mut esp_nvs::Nvs<&mut common::Flash>,
            multi_chunk: &[u8],
        ) -> Result<(), esp_nvs::error::Error> {
            let ns = Key::from_str("ns1");
            let key = Key::from_str("k");
            match self {
                Value::Primitive => nvs.set(&ns, &key, 42u8),
                Value::Str => nvs.set(&ns, &key, "short string"),
                Value::SingleChunkBlob => nvs.set(&ns, &key, [7u8; 64].as_slice()),
                Value::MultiChunkBlob => nvs.set(&ns, &key, multi_chunk),
            }
        }

        fn write(self, nvs: &mut esp_nvs::Nvs<&mut common::Flash>, multi_chunk: &[u8]) {
            self.try_write(nvs, multi_chunk)
                .unwrap_or_else(|e| panic!("writing {self:?} failed: {e:?}"));
        }

        fn is_readable(self, nvs: &mut esp_nvs::Nvs<&mut common::Flash>, multi_chunk: &[u8]) -> bool {
            let ns = Key::from_str("ns1");
            let key = Key::from_str("k");
            match self {
                Value::Primitive => nvs.get::<u8>(&ns, &key) == Ok(42),
                Value::Str => nvs.get::<String>(&ns, &key).as_deref() == Ok("short string"),
                Value::SingleChunkBlob => nvs.get::<Vec<u8>>(&ns, &key) == Ok(vec![7u8; 64]),
                Value::MultiChunkBlob => nvs.get::<Vec<u8>>(&ns, &key) == Ok(multi_chunk.to_vec()),
            }
        }

        fn assert_readable(self, nvs: &mut esp_nvs::Nvs<&mut common::Flash>, multi_chunk: &[u8], when: &str) {
            assert!(self.is_readable(nvs, multi_chunk), "{self:?} did not read back {when}");
        }
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

    /// Overwrites a counter often enough that even the oldest page of a three page partition goes
    /// through defragmentation. That takes a while, since a page with fewer erased entries only
    /// wins once it is old enough.
    fn churn<T: esp_nvs::platform::Platform>(nvs: &mut esp_nvs::Nvs<T>) {
        for value in 0..6_000u32 {
            nvs.set(&Key::from_str("ns1"), &Key::from_str("counter"), value)
                .unwrap();
        }
    }

    /// A string whose data fails its CRC has to stay unreadable through a defragmentation.
    /// Copying rewrote the data with a freshly computed CRC, so the corrupt value came out of it
    /// as a valid one.
    #[test]
    fn a_corrupt_string_stays_corrupt_through_defragmentation() {
        let mut flash = common::Flash::new(3);
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set(&Key::from_str("ns1"), &Key::from_str("s"), "hello hello hello")
                .unwrap();
        }
        // Entry 1 is the string's header, entry 2 its data.
        flash.buf[common::ITEM_OFFSET + 2 * esp_nvs::ITEM_SIZE] = 0x00;

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert!(nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("s")).is_err());
        churn(&mut nvs);
        assert!(nvs.get::<String>(&Key::from_str("ns1"), &Key::from_str("s")).is_err());
    }

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
        flash.arm_fault(99);

        {
            // set() will trigger defragmentation and fail during the copy phase
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            let result = nvs.set(&Key::from_str("ns1"), &Key::from_str("trigger_defrag"), 255u8);
            assert_eq!(result, Err(FlashError));
        }

        // the last successful operation was write an item to the new page (each copied item is a
        // read from the source, a write to the target and an entry map update)
        assert_eq!(
            flash.operations.last().unwrap(),
            &Operation::Write { offset: 4896, len: 32 }
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
        flash.arm_fault(195);

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

    /// A blob whose chunks are spread over several pages has to be scrubbed in full. The
    /// single-page test above cannot tell whether the sweep reaches past the page holding the blob
    /// index, and a chunk left behind on another page is exactly what purge mode exists to prevent.
    ///
    /// The payload is checked by searching the whole partition for it rather than by naming entry
    /// offsets, since the chunks do not sit at a fixed place once they cross a page boundary.
    #[test]
    fn continuous_purge_zeroes_a_multi_page_blob() {
        // A pattern that does not occur in a page header, an entry state bitmap or an item header,
        // so finding it anywhere means finding blob payload.
        let payload: Vec<u8> = [0xA5u8, 0x5A].iter().copied().cycle().take(9000).collect();

        let mut flash = common::Flash::new(8);
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set_purge_mode(true);
            nvs.set(&Key::from_str("ns1"), &Key::from_str("secret"), payload.as_slice())
                .unwrap();

            // The blob really does span pages, otherwise this repeats the test above.
            let per_page = &nvs.statistics().unwrap().entries_per_page;
            let pages_with_data = per_page.iter().filter(|p| p.written > 0).count();
            assert!(
                pages_with_data >= 3,
                "expected the blob to span pages, got {pages_with_data}"
            );
        }

        assert!(
            contains(&flash.buf, &payload[..64]),
            "the payload should be on flash before it is deleted"
        );

        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            nvs.set_purge_mode(true);
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("secret")).unwrap();
        }

        // Not one chunk of it is left anywhere in the partition, on any page.
        assert!(
            !contains(&flash.buf, &payload[..64]),
            "a chunk of the purged blob is still on flash"
        );
        // Nor a single entry's worth of it, which a partial sweep would leave.
        assert!(!contains(&flash.buf, &payload[..ITEM_SIZE]));

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("secret"))
                .err()
                .unwrap(),
            KeyNotFound
        );
    }

    /// The counterpart, so the search above is known to be capable of finding something: without
    /// purge mode the payload stays on flash after the delete.
    #[test]
    fn default_mode_leaves_a_multi_page_blob_in_flash() {
        let payload: Vec<u8> = [0xA5u8, 0x5A].iter().copied().cycle().take(9000).collect();

        let mut flash = common::Flash::new(8);
        {
            let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
            assert!(!nvs.purge_mode());
            nvs.set(&Key::from_str("ns1"), &Key::from_str("secret"), payload.as_slice())
                .unwrap();
            nvs.delete(&Key::from_str("ns1"), &Key::from_str("secret")).unwrap();
        }

        assert!(
            contains(&flash.buf, &payload[..64]),
            "deleting without purge mode should only flip the entry state"
        );
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
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

/// A blob version owns one half of the chunk index space, picked by the `chunk_start` recorded in
/// its index: 0x00 or 0x80. Successive versions have to take turns, because the old version's
/// chunks are only deleted once the new index has been written - if a new version reused the base
/// its chunks would be written over the indices the old ones still occupy, and a crash in between
/// would leave a blob assembled from both.
mod blob_versions {
    use esp_nvs::{
        ENTRIES_PER_PAGE,
        FLASH_SECTOR_SIZE,
        ITEM_SIZE,
        ItemType,
        Key,
    };
    use pretty_assertions::assert_eq;

    use crate::common;

    /// `ItemDataBlobIndex` is `{ size: u32, chunk_count: u8, chunk_start: u8 }` over the data
    /// union.
    const BLOB_INDEX_CHUNK_START_OFFSET: usize = common::ITEM_DATA_OFFSET + 5;

    /// The `chunk_start` of every live blob index on flash.
    ///
    /// Both filters matter. Without the entry state check an erased index from a previous version
    /// is picked up alongside the live one, and without the item CRC check a blob payload byte
    /// that happens to sit where a type byte would go is mistaken for an index - which is
    /// exactly what this returned before the CRC check was added, a `chunk_start` of 100.
    fn live_blob_index_chunk_starts(buf: &[u8]) -> Vec<u8> {
        let mut found = vec![];
        for page_start in (0..buf.len()).step_by(FLASH_SECTOR_SIZE) {
            for entry in 0..ENTRIES_PER_PAGE {
                if common::entry_state(buf, page_start, entry) != common::ENTRY_STATE_WRITTEN {
                    continue;
                }
                let offset = page_start + common::ITEM_OFFSET + entry * ITEM_SIZE;
                if buf[offset + 1] == ItemType::BlobIndex as u8 && common::is_item_header(buf, offset) {
                    found.push(buf[offset + BLOB_INDEX_CHUNK_START_OFFSET]);
                }
            }
        }
        found
    }

    #[test]
    fn successive_blob_versions_alternate_their_chunk_base() {
        let mut flash = common::Flash::new(8);

        // Large enough to need several chunks, so the base is what keeps the two versions apart
        // rather than there being room for both at different indices anyway.
        let versions: Vec<Vec<u8>> = (0u8..6).map(|i| (i..=255).cycle().take(9000).collect()).collect();

        let mut seen = vec![];
        for (i, value) in versions.iter().enumerate() {
            {
                let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
                nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), value.as_slice())
                    .unwrap();
                assert_eq!(
                    nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b")).unwrap(),
                    *value,
                    "version {i} did not read back"
                );
            }

            let starts = live_blob_index_chunk_starts(&flash.buf);
            assert_eq!(
                starts.len(),
                1,
                "version {i}: expected exactly one live blob index, got {starts:?}"
            );
            seen.push(starts[0]);
        }

        // The first version starts at 0x00 and every one after it flips.
        assert_eq!(seen, vec![0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
    }

    /// A blob write that fails part way must not leave chunks behind at the indices the next
    /// attempt uses.
    ///
    /// Both attempts take the same version base, and `load_item` finds the leftover chunk first, so
    /// the next blob of the key read back as `CorruptedData`. At the following boot its chunks no
    /// longer added up and it was deleted, so the key was gone.
    #[test]
    fn a_failed_blob_write_leaves_no_chunks_behind() {
        let flash = common::SharedFlash::new(4);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        nvs.set(
            &Key::from_str("ns1"),
            &Key::from_str("filler"),
            [0xAAu8; 4000].as_slice(),
        )
        .unwrap();
        assert_eq!(
            nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), [0xBBu8; 9000].as_slice()),
            Err(esp_nvs::error::Error::FlashFull)
        );

        nvs.delete(&Key::from_str("ns1"), &Key::from_str("filler")).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), [0x11u8; 100].as_slice())
            .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b")),
            Ok(vec![0x11u8; 100])
        );
        drop(nvs);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b")),
            Ok(vec![0x11u8; 100])
        );
    }

    /// A blob whose chunk no longer reads back has to be replaceable by writing it again.
    ///
    /// The check for an unchanged value read the old blob first and passed the read error on, so
    /// `set` failed with `KeyNotFound` and the blob stayed broken until a reboot.
    #[test]
    fn a_corrupt_blob_can_be_overwritten() {
        let flash = common::SharedFlash::new(4);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), [1u8; 100].as_slice())
            .unwrap();

        // Entry 0 is the namespace, entry 1 the chunk header. Break its CRC via a key bit.
        flash.with_buf(|buf| buf[common::ITEM_OFFSET + ITEM_SIZE + 8] ^= 0x01);
        assert!(nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b")).is_err());

        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), [2u8; 100].as_slice())
            .unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b")),
            Ok(vec![2u8; 100])
        );
        drop(nvs);

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), flash.clone()).unwrap();
        assert_eq!(
            nvs.get::<Vec<u8>>(&Key::from_str("ns1"), &Key::from_str("b")),
            Ok(vec![2u8; 100])
        );
    }

    /// The data field of a blob index is 8 bytes, of which size, chunk count and chunk start take
    /// 6. ESP-IDF leaves the last two unprogrammed. They were left uninitialized instead, so
    /// whatever memory held went to flash and into the item CRC.
    #[test]
    fn a_blob_index_leaves_its_reserved_bytes_unprogrammed() {
        let mut flash = common::Flash::new(3);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), [1u8; 100].as_slice())
            .unwrap();
        drop(nvs);

        let index = (0..ENTRIES_PER_PAGE)
            .map(|entry| common::ITEM_OFFSET + entry * ITEM_SIZE)
            .find(|&offset| {
                flash.buf[offset + 1] == ItemType::BlobIndex as u8 && common::is_item_header(&flash.buf, offset)
            })
            .unwrap();
        assert_eq!(
            flash.buf[index + common::ITEM_DATA_OFFSET + 6..index + ITEM_SIZE],
            [0xFF, 0xFF]
        );
    }

    /// A blob written fresh after its predecessor was deleted starts over at the first base, since
    /// there is no live version left to differ from.
    #[test]
    fn a_blob_written_after_a_delete_starts_at_the_first_base() {
        let mut flash = common::Flash::new(8);
        let payload: Vec<u8> = (0u8..=255).cycle().take(9000).collect();

        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), payload.as_slice())
            .unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), payload.as_slice())
            .unwrap();
        nvs.delete(&Key::from_str("ns1"), &Key::from_str("b")).unwrap();
        nvs.set(&Key::from_str("ns1"), &Key::from_str("b"), payload.as_slice())
            .unwrap();
        drop(nvs);

        assert_eq!(live_blob_index_chunk_starts(&flash.buf), vec![0x00]);
    }
}

mod namespaces {
    use esp_nvs::Key;
    use esp_nvs::error::Error;
    use pretty_assertions::assert_eq;

    use crate::common;

    /// ESP-IDF reserves namespace index 255 to match any namespace, so a partition can hold 254.
    /// The 255th used to be given index 255, which ESP-IDF then reads as a wildcard.
    #[test]
    fn at_most_254_namespaces_are_created() {
        let mut flash = common::Flash::new(6);
        let mut nvs = esp_nvs::Nvs::new(0, flash.len(), &mut flash).unwrap();
        for i in 0..254u32 {
            nvs.set(&Key::from_str(&format!("n{i}")), &Key::from_str("k"), i)
                .unwrap();
        }
        assert_eq!(
            nvs.set(&Key::from_str("one_too_many"), &Key::from_str("k"), 1u32),
            Err(Error::TooManyNamespaces)
        );
        for i in 0..254u32 {
            assert_eq!(
                nvs.get::<u32>(&Key::from_str(&format!("n{i}")), &Key::from_str("k")),
                Ok(i)
            );
        }
    }
}
