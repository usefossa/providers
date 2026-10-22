use fossa_sdk::{
    accounts::*,
    address,
    amount::*,
    instruction::*,
    metadata::{Document, Kind},
    receipt::*,
    *,
};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn context() -> (Protocol, ModelKeys, UserKeys, OfferKeys, SettlementKeys) {
    let p = Protocol {
        program: key(1),
        market: key(2),
        mint: key(3),
    };
    let m = ModelKeys {
        model: address::model(&p.program, &p.market, &[4; 32]).0,
        terms: key(5),
        capacity: key(6),
    };
    let u = UserKeys {
        user: key(7),
        owner: key(8),
        vault: key(9),
    };
    let o = OfferKeys {
        model: m,
        provider: key(10),
        payout: key(11),
        bond_vault: key(12),
        slot: 2,
    };
    (
        p,
        m,
        u,
        o,
        SettlementKeys {
            offer: o,
            user: u,
            booking: key(13),
        },
    )
}
fn terms() -> OfferTerms {
    OfferTerms {
        rates: Rates {
            input: 1_234_567,
            cached: 345_678,
            output: 9_876_543,
        },
        speed_tps: 300,
        private: true,
        active: true,
        capacity: 5,
    }
}
fn request() -> BookRequest {
    BookRequest {
        model_id: [4; 32],
        rate_caps: Rates {
            input: u32::MAX,
            cached: u32::MAX,
            output: u32::MAX,
        },
        min_speed: 1,
        max_input: 100,
        max_output: 50,
        private: false,
        max_spend: 10_000,
        estimated_input: 20,
        estimated_cached: 5,
        estimated_output: 10,
        cache_provider: key(10),
    }
}
fn doc() -> DocumentRef {
    DocumentRef::new([42; 32], "https://example.com/m.json").unwrap()
}
fn put(d: &mut [u8], at: usize, bytes: &[u8]) {
    d[at..at + bytes.len()].copy_from_slice(bytes);
}
fn raw(program: Pubkey, magic: &[u8; 8], len: usize) -> RawAccount {
    let mut data = vec![0; len];
    data[..8].copy_from_slice(magic);
    RawAccount {
        owner: program,
        lamports: 123,
        executable: false,
        data,
    }
}

#[test]
fn instruction_account_order_flags_and_payload_lengths_cover_every_opcode() {
    let (p, m, u, o, k) = context();
    let d = doc();
    let receipt = Receipt {
        owner: u.owner,
        user: u.user,
        booking: k.booking,
        generation: 1,
        charge: 1,
        dispute_end: 100,
    };
    let proof = ReceiptProof {
        epoch: 10,
        index: 0,
        receipt,
        siblings: vec![],
    };
    let instructions = vec![
        p.initialize_market(key(20)),
        p.initialize_user(u),
        p.register_offer(o, terms(), None).unwrap(),
        p.deposit(u, key(21), 1).unwrap(),
        p.fund_bond(o, key(21), 1).unwrap(),
        p.update_offer(o, terms()).unwrap(),
        p.book(m, u, k.booking, key(22), &request(), BookingMode::OneShot)
            .unwrap(),
        p.claim(k, Usage::default(), None).unwrap(),
        p.expire(k, None).unwrap(),
        p.finalize(m, k.booking),
        p.withdraw(u, key(21), 1).unwrap(),
        p.dispute(m, k.booking, u.owner),
        p.resolve(k, key(20), 1),
        p.request_unbond(o, key(23), key(22), 1).unwrap(),
        p.execute_unbond(o, key(23)),
        p.release(k, None).unwrap(),
        p.approve_offer(m, 2, key(20), true),
        p.close(k.booking, key(22), false),
        p.initialize_model(InitializeModel {
            model: m,
            provider: o.provider,
            payer: key(22),
            admin: None,
            id: [4; 32],
            limit: 8,
            document: &d,
        })
        .unwrap(),
        p.initialize_provider(o.provider, key(22), &d).unwrap(),
        p.approve_provider(o.provider, key(20), true),
        p.update_provider_metadata(o.provider, &d).unwrap(),
        p.update_model_metadata(m.model, o.provider, &d).unwrap(),
        p.initialize_receipt_bucket(
            m,
            InitializeReceiptBucket {
                bucket: key(24),
                payer: key(22),
                slot: 2,
                duration: 60,
                epoch: None,
                fresh: true,
            },
        )
        .unwrap(),
        p.finalize_receipt_bucket(m, key(24)),
        p.dispute_receipt(m.model, key(24), key(22), &proof)
            .unwrap(),
        p.initialize_reusable_booking(u, k.booking, key(22)),
        p.close_receipt_bucket(key(24), key(22), o.provider),
    ];
    let layouts: &[(usize, &[usize], &[usize])] = &[
        (3, &[0], &[0, 1]),
        (4, &[0], &[0, 1]),
        (8, &[2, 3], &[5]),
        (7, &[1, 3, 4], &[2]),
        (9, &[3, 5, 6], &[4]),
        (4, &[2], &[3]),
        (8, &[3, 4, 6], &[5, 6, 7]),
        (11, &[2, 3, 4, 6, 7], &[5]),
        (5, &[2, 3, 4], &[]),
        (4, &[2, 3], &[]),
        (8, &[1, 3, 4], &[2]),
        (4, &[2], &[3]),
        (11, &[2, 3, 4, 6, 7], &[5]),
        (7, &[3, 5], &[4, 5, 6]),
        (11, &[3, 4, 6, 7], &[5]),
        (6, &[2, 3, 4], &[5]),
        (4, &[2], &[3]),
        (2, &[0, 1], &[]),
        (9, &[1, 2, 3, 5, 8], &[2, 3, 4, 5]),
        (5, &[1, 3], &[2, 3]),
        (3, &[1], &[2]),
        (3, &[1], &[2]),
        (4, &[2], &[3]),
        (5, &[3], &[3, 4]),
        (4, &[2, 3], &[]),
        (7, &[2, 3, 5], &[4, 5]),
        (5, &[3], &[2, 3, 4]),
        (3, &[0, 1], &[2]),
    ];
    let lengths = [
        1,
        1,
        25,
        9,
        13,
        25,
        108,
        13,
        1,
        1,
        9,
        1,
        9,
        13,
        1,
        1,
        6,
        1,
        69 + d.uri.len(),
        33 + d.uri.len(),
        2,
        33 + d.uri.len(),
        33 + d.uri.len(),
        9,
        1,
        137,
        1,
        1,
    ];
    for (op, ix) in instructions.iter().enumerate() {
        assert_eq!(ix.program_id, p.program);
        assert_eq!(ix.data[0], op as u8);
        assert_eq!(ix.data.len(), lengths[op], "opcode {op}");
        let (count, writable, signers) = layouts[op];
        assert_eq!(ix.accounts.len(), count, "opcode {op}");
        for (index, a) in ix.accounts.iter().enumerate() {
            assert_eq!(
                a.is_writable,
                writable.contains(&index),
                "opcode {op}, writable {index}"
            );
            assert_eq!(
                a.is_signer,
                signers.contains(&index),
                "opcode {op}, signer {index}"
            );
        }
    }
    assert_eq!(
        instructions[3]
            .accounts
            .iter()
            .map(|a| a.pubkey)
            .collect::<Vec<_>>(),
        [
            p.market,
            u.user,
            u.owner,
            key(21),
            u.vault,
            p.mint,
            address::TOKEN_PROGRAM
        ]
    );
    assert_eq!(&instructions[2].data[1..5], &2u32.to_le_bytes());
    assert_eq!(&instructions[2].data[5..9], &1_234_567u32.to_le_bytes());
    assert_eq!(&instructions[2].data[17..21], &[44, 1, 1, 1]);
    assert_eq!(&instructions[2].data[21..], &5u32.to_le_bytes());
    assert_eq!(
        instructions[12].accounts[9].pubkey,
        address::bond(&p.program, &m.model, 2).0
    );
    assert_eq!(
        instructions[25].accounts[3].pubkey,
        address::receipt_case(&p.program, &key(24), 10, 0).0
    );
}

#[test]
fn reusable_variants_and_booking_fields_match_active_layout() {
    let (p, m, u, _, k) = context();
    let q = request();
    let ix = p
        .book(
            m,
            u,
            k.booking,
            key(22),
            &q,
            BookingMode::Reusable {
                generation: 7,
                fresh: false,
            },
        )
        .unwrap();
    assert_eq!(ix.data.len(), 116);
    assert!(!ix.accounts[6].is_signer);
    assert!(!ix.accounts[7].is_signer);
    assert_eq!(&ix.data[1..33], &q.model_id);
    assert_eq!(&ix.data[47..51], &100u32.to_le_bytes());
    assert_eq!(&ix.data[56..64], &10_000u64.to_le_bytes());
    assert_eq!(&ix.data[76..108], q.cache_provider.as_ref());
    assert_eq!(&ix.data[108..116], &7u64.to_le_bytes());
    let receipt = Receipt {
        owner: u.owner,
        user: u.user,
        booking: k.booking,
        generation: 7,
        charge: 89,
        dispute_end: 456,
    };
    let published = ClaimReceipt {
        bucket: key(24),
        epoch: 123,
        index: 5,
        receipt: receipt.clone(),
    };
    let ix = p
        .claim(
            k,
            Usage {
                uncached: 10,
                cached: 3,
                output: 4,
            },
            Some(&published),
        )
        .unwrap();
    assert_eq!(ix.data.len(), 157);
    assert_eq!(&ix.data[13..21], &7u64.to_le_bytes());
    assert_eq!(&ix.data[21..29], &123i64.to_le_bytes());
    assert_eq!(&ix.data[37..], &receipt.encode());
    let decoded = PublishedReceipt::from_claim(&p.program, &ix).unwrap();
    assert_eq!(decoded.receipt, receipt);
    assert_eq!(decoded.index, 5);
    let mut wrong = ix;
    wrong.data[13] = 8;
    assert!(PublishedReceipt::from_claim(&p.program, &wrong).is_err());
    assert_eq!(
        p.expire(k, Some(7)).unwrap().data,
        [8, 7, 0, 0, 0, 0, 0, 0, 0]
    );
    assert!(p.release(k, Some(0)).is_err());
    assert!(p.close(k.booking, key(22), true).accounts[1].is_signer);
}

#[test]
fn exact_atomic_math_rounding_boundaries_and_overflow() {
    assert_eq!(parse_usdc("18446744073709.551615").unwrap(), u64::MAX);
    for invalid in [
        "18446744073709.551616",
        "-1",
        "+1",
        "1e3",
        "1.0000001",
        "NaN",
        " 1",
        ".1",
    ] {
        assert!(parse_usdc(invalid).is_err(), "{invalid}");
    }
    assert_eq!(parse_usdc("0.000001").unwrap(), 1);
    assert_eq!(format_usdc(u64::MAX), "18446744073709.551615");
    assert_eq!(fee(1, 1), 1);
    assert_eq!(fee(0, u32::MAX), 0);
    assert_eq!(fee(u32::MAX, u32::MAX), 18_446_744_065_120);
    assert_eq!(collateral(1).unwrap(), 2);
    assert_eq!(collateral(100).unwrap(), 101);
    assert_eq!(collateral(101).unwrap(), 103);
    assert!(collateral(u64::MAX).is_err());
    assert_eq!(
        Rates {
            input: 1,
            cached: 1,
            output: 1
        }
        .charge(Usage {
            uncached: 1,
            cached: 1,
            output: 1
        }),
        3
    );
    assert_eq!(
        Rates {
            input: 1,
            cached: 0,
            output: 1
        }
        .reserve(1, 1),
        3
    );
}

#[test]
fn matching_uses_full_caps_cache_identity_collateral_and_stable_ties() {
    let (_, m, _, _, _) = context();
    let t = OfferTerms {
        rates: Rates {
            input: 1_000_000,
            cached: 1,
            output: 1_000_000,
        },
        speed_tps: 10,
        private: true,
        active: true,
        capacity: 1,
    };
    let offer = |provider| Offer {
        provider,
        payout: key(11),
        bond_vault: key(12),
        terms: t,
        approved: true,
        bond_bump: 0,
    };
    let mut terms = TermsBook {
        model: m.model,
        limit: 8,
        entries: vec![offer(key(10)), offer(key(11))],
    };
    let cap = Capacity {
        funded_bond: 10_000,
        ..Default::default()
    };
    let mut capacity = CapacityBook {
        model: m.model,
        limit: 8,
        entries: vec![cap, cap],
    };
    let mut q = request();
    assert_eq!(
        q.select(&terms, &capacity, 10_000).unwrap().unwrap().slot,
        0
    );
    q.cache_provider = key(11);
    assert_eq!(
        q.select(&terms, &capacity, 10_000).unwrap().unwrap().slot,
        1
    );
    q.estimated_cached = 0;
    assert_eq!(
        q.select(&terms, &capacity, 10_000).unwrap().unwrap().slot,
        0
    );
    q.rate_caps.cached = 0;
    assert!(q.select(&terms, &capacity, 10_000).unwrap().is_none());
    q.rate_caps.cached = u32::MAX;
    capacity.entries[0].exposure = u64::MAX;
    assert_eq!(
        q.select(&terms, &capacity, 10_000).unwrap().unwrap().slot,
        1
    );
    capacity.entries[1].funded_bond = 151;
    assert!(q.select(&terms, &capacity, 10_000).unwrap().is_none());
    capacity.entries[1].funded_bond = 153;
    assert_eq!(
        q.select(&terms, &capacity, 10_000)
            .unwrap()
            .unwrap()
            .reserve,
        151
    );
    terms.entries[1].terms.active = false;
    assert!(q.select(&terms, &capacity, 10_000).unwrap().is_none());
}

#[test]
fn metadata_commits_exact_bytes_and_rejects_bad_model_binding() {
    let bytes =
        b"{\n  \"name\": \"Example\", \"endpoint\": \"https://example.com/infer\"\n}\n".to_vec();
    let d = Document::parse(Kind::Provider, bytes.clone()).unwrap();
    assert_eq!(d.bytes(), bytes);
    let compact = Document::parse(
        Kind::Provider,
        br#"{"name":"Example","endpoint":"https://example.com/infer"}"#.to_vec(),
    )
    .unwrap();
    assert_ne!(d.sha256(), compact.sha256());
    d.verify(&d.reference("https://example.com/a.json").unwrap())
        .unwrap();
    assert!(
        compact
            .verify(&d.reference("https://example.com/a.json").unwrap())
            .is_err()
    );
    assert!(Document::model("wrong",br#"{"id":"test","name":"Test","modalities":{"input":["text"],"output":["text"]},"contextTokens":100,"outputTokens":10}"#.to_vec()).is_err());
    for uri in [
        "",
        "https://example.com/a b",
        "https://example.com/\n",
        "https://example.com/\u{7f}",
    ] {
        assert!(DocumentRef::new([0; 32], uri).is_err());
    }
}

#[test]
fn account_decoders_validate_owner_layout_pdas_bindings_and_padding() {
    let (p, m, _, _, _) = context();
    let mut model = raw(p.program, Model::MAGIC, MODEL_LEN);
    put(&mut model.data, 8, p.market.as_ref());
    put(&mut model.data, 40, &[4; 32]);
    put(&mut model.data, 72, m.terms.as_ref());
    put(&mut model.data, 104, m.capacity.as_ref());
    put(&mut model.data, 136, &2u32.to_le_bytes());
    model.data[140] = address::model(&p.program, &p.market, &[4; 32]).1;
    let decoded = Model::decode(&m.model, &model, &p.program).unwrap();
    assert_eq!(decoded.limit, 2);
    assert!(Model::decode(&key(99), &model, &p.program).is_err());
    for len in 0..MODEL_LEN {
        let mut bad = model.clone();
        bad.data.truncate(len);
        assert!(Model::decode(&m.model, &bad, &p.program).is_err());
    }
    let mut wrong = model.clone();
    wrong.owner = key(99);
    assert!(Model::decode(&m.model, &wrong, &p.program).is_err());
    wrong = model.clone();
    wrong.data[140] ^= 1;
    assert!(Model::decode(&m.model, &wrong, &p.program).is_err());
    let authority = key(10);
    let (profile, bump) = address::provider(&p.program, &p.market, &authority);
    let mut account = raw(p.program, Provider::MAGIC, PROVIDER_LEN);
    put(&mut account.data, 8, p.market.as_ref());
    put(&mut account.data, 40, authority.as_ref());
    put(&mut account.data, 72, &[42; 32]);
    put(&mut account.data, 104, &1u16.to_le_bytes());
    account.data[106] = 1;
    account.data[107] = bump;
    account.data[108] = b'x';
    assert!(
        Provider::decode(&profile, &account, &p.program)
            .unwrap()
            .approved
    );
    account.data[109] = 1;
    assert!(Provider::decode(&profile, &account, &p.program).is_err());
    account.data[109] = 0;
    account.data[106] = 2;
    assert!(Provider::decode(&profile, &account, &p.program).is_err());
    let mut capacity = raw(p.program, CapacityBook::MAGIC, 48 + 2 * 32);
    put(&mut capacity.data, 8, m.model.as_ref());
    put(&mut capacity.data, 40, &2u32.to_le_bytes());
    put(&mut capacity.data, 44, &1u32.to_le_bytes());
    put(&mut capacity.data, 48, &u64::MAX.to_le_bytes());
    assert_eq!(
        CapacityBook::decode(&m.capacity, &capacity, &p.program)
            .unwrap()
            .entries[0]
            .funded_bond,
        u64::MAX
    );
    put(&mut capacity.data, 44, &3u32.to_le_bytes());
    assert!(CapacityBook::decode(&m.capacity, &capacity, &p.program).is_err());
    let mut booking = raw(p.program, Booking::MAGIC, BOOKING_LEN);
    booking.data[148] = 5;
    assert_eq!(
        Booking::decode(&key(13), &booking, &p.program)
            .unwrap()
            .status,
        BookingStatus::Available
    );
    booking.data[148] = 6;
    assert!(Booking::decode(&key(13), &booking, &p.program).is_err());
}

#[test]
fn receipt_merkle_proofs_bind_program_bucket_epoch_index_and_payload() {
    let program = key(1);
    let bucket = key(2);
    let receipts = (0..13)
        .map(|i| Receipt {
            owner: key(3),
            user: key(4),
            booking: key(5),
            generation: i + 1,
            charge: 10 + i,
            dispute_end: 1000,
        })
        .collect::<Vec<_>>();
    let state = ReceiptBucket {
        market: key(6),
        model: key(7),
        provider: key(8),
        bond_vault: key(9),
        rent_payer: key(10),
        slot: 0,
        bond_bump: 0,
        finalized: false,
        epoch: 100,
        epoch_end: 200,
        dispute_end: 1000,
        count: 13,
        charge: 208,
        outstanding: 13,
        frontier: frontier(&program, &bucket, 100, &receipts).unwrap(),
    };
    for index in 0..13 {
        let proof = ReceiptProof::build(&program, &bucket, 100, &receipts, index).unwrap();
        proof.verify(&program, &bucket, &state).unwrap();
        let mut bad = proof.clone();
        bad.receipt.charge += 1;
        assert!(bad.verify(&program, &bucket, &state).is_err());
        assert!(proof.verify(&key(11), &bucket, &state).is_err());
        assert!(proof.verify(&program, &key(11), &state).is_err());
    }
    assert_eq!(
        ReceiptProof::build(&program, &bucket, 100, &receipts, 12)
            .unwrap()
            .siblings
            .len(),
        0
    );
    assert!(ReceiptProof::build(&program, &bucket, 100, &receipts, 13).is_err());
    let mut proof = ReceiptProof::build(&program, &bucket, 100, &receipts, 0).unwrap();
    proof.siblings[0][0] ^= 1;
    assert!(proof.verify(&program, &bucket, &state).is_err());
}

#[test]
fn invalid_builders_fail_before_rpc_or_signing() {
    let (p, m, u, o, k) = context();
    assert!(p.deposit(u, key(21), 0).is_err());
    assert!(p.fund_bond(o, key(21), 0).is_err());
    assert!(
        p.register_offer(
            o,
            OfferTerms {
                speed_tps: 0,
                ..terms()
            },
            None
        )
        .is_err()
    );
    assert!(
        p.update_offer(
            o,
            OfferTerms {
                rates: Rates {
                    input: 1,
                    cached: 2,
                    output: 1
                },
                ..terms()
            }
        )
        .is_err()
    );
    let mut q = request();
    q.estimated_cached = 21;
    assert!(
        p.book(m, u, k.booking, key(22), &q, BookingMode::OneShot)
            .is_err()
    );
    q = request();
    q.model_id = [0; 32];
    assert!(
        p.book(m, u, k.booking, key(22), &q, BookingMode::OneShot)
            .is_err()
    );
    assert!(book_sizes(0).is_err());
    assert!(book_sizes(4097).is_err());
}

#[test]
fn pda_vectors_match_independent_web3_derivation() {
    assert_eq!(
        address::provider(&key(1), &key(2), &key(3)).0.to_string(),
        "Gt4b1itRCfTy9dLS4Sy8erP7x2jsiUQvKQm8m5xL97WW"
    );
    assert_eq!(
        address::model(&key(1), &key(2), &[4; 32]).0.to_string(),
        "7A8Fqk1FXhwkh93nk1R6CjzhBo9k7dKLqATEpUJY8rSW"
    );
    assert_eq!(
        address::vault(&key(1), &key(3)).0.to_string(),
        "J2FbwMp5fGYX6Zng12uULs2PFoeDjn4EiXyyPwrENBFt"
    );
}

#[test]
fn remaining_accounts_decode_exact_fields_and_reject_malformed_records() {
    let (p, m, u, _, _) = context();
    let mut market = raw(p.program, Market::MAGIC, MARKET_LEN);
    put(&mut market.data, 8, key(10).as_ref());
    put(&mut market.data, 40, p.mint.as_ref());
    assert_eq!(
        Market::decode(&p.market, &market, &p.program).unwrap().mint,
        p.mint
    );
    let mut user = raw(p.program, User::MAGIC, USER_LEN);
    put(&mut user.data, 8, u.owner.as_ref());
    put(&mut user.data, 40, p.market.as_ref());
    put(&mut user.data, 72, &u64::MAX.to_le_bytes());
    put(&mut user.data, 80, u.vault.as_ref());
    user.data[112] = address::vault(&p.program, &u.user).1;
    user.data[113..120].fill(255);
    let decoded = User::decode(&u.user, &user, &p.program).unwrap();
    assert_eq!(decoded.generation, (1 << 56) - 1);
    assert_eq!(decoded.free, u64::MAX);
    user.data[112] ^= 1;
    assert!(User::decode(&u.user, &user, &p.program).is_err());
    let (metadata, bump) = address::model_metadata(&p.program, &m.model);
    let mut account = raw(p.program, ModelMetadata::MAGIC, MODEL_METADATA_LEN);
    put(&mut account.data, 8, p.market.as_ref());
    put(&mut account.data, 40, m.model.as_ref());
    put(&mut account.data, 72, key(10).as_ref());
    put(&mut account.data, 104, &[42; 32]);
    put(&mut account.data, 136, &1u16.to_le_bytes());
    account.data[138] = bump;
    account.data[140] = b'x';
    assert_eq!(
        ModelMetadata::decode(&metadata, &account, &p.program)
            .unwrap()
            .document
            .sha256,
        [42; 32]
    );
    account.data[138] ^= 1;
    assert!(ModelMetadata::decode(&metadata, &account, &p.program).is_err());
    let mut unbond = raw(p.program, Unbond::MAGIC, UNBOND_LEN);
    put(&mut unbond.data, 8, p.market.as_ref());
    put(&mut unbond.data, 40, key(10).as_ref());
    put(&mut unbond.data, 72, &100u64.to_le_bytes());
    put(&mut unbond.data, 80, &123i64.to_le_bytes());
    unbond.data[88] = 1;
    put(&mut unbond.data, 92, &2u32.to_le_bytes());
    put(&mut unbond.data, 96, m.model.as_ref());
    put(&mut unbond.data, 128, key(22).as_ref());
    let v = Unbond::decode(&key(23), &unbond, &p.program).unwrap();
    assert!(v.executed);
    assert_eq!(v.ready, 123);
    assert_eq!(v.amount, 100);
    unbond.data[88] = 2;
    assert!(Unbond::decode(&key(23), &unbond, &p.program).is_err());
    let mut terms_account = raw(p.program, TermsBook::MAGIC, HEADER_LEN + TERMS_LEN);
    put(&mut terms_account.data, 8, m.model.as_ref());
    put(&mut terms_account.data, 40, &1u32.to_le_bytes());
    put(&mut terms_account.data, 44, &1u32.to_le_bytes());
    put(&mut terms_account.data, 48, key(10).as_ref());
    put(&mut terms_account.data, 80, key(11).as_ref());
    put(&mut terms_account.data, 112, key(12).as_ref());
    let bytes = terms().encode().unwrap();
    put(&mut terms_account.data, 144, &bytes[..16]);
    terms_account.data[160] = 1;
    terms_account.data[161] = address::bond(&p.program, &m.model, 0).1;
    put(&mut terms_account.data, 164, &bytes[16..]);
    assert_eq!(
        TermsBook::decode(&m.terms, &terms_account, &p.program)
            .unwrap()
            .entries[0]
            .terms,
        terms()
    );
    terms_account.data[160] = 2;
    assert!(TermsBook::decode(&m.terms, &terms_account, &p.program).is_err());
    let mut bucket = raw(p.program, ReceiptBucket::MAGIC, RECEIPT_BUCKET_LEN);
    put(&mut bucket.data, 8, p.market.as_ref());
    put(&mut bucket.data, 40, m.model.as_ref());
    bucket.data[172] = address::bond(&p.program, &m.model, 0).1;
    put(&mut bucket.data, 176, &100i64.to_le_bytes());
    put(&mut bucket.data, 184, &200i64.to_le_bytes());
    put(&mut bucket.data, 200, &13u64.to_le_bytes());
    put(&mut bucket.data, 208, &99u64.to_le_bytes());
    put(&mut bucket.data, 216, &3u64.to_le_bytes());
    assert_eq!(
        ReceiptBucket::decode(&key(24), &bucket, &p.program)
            .unwrap()
            .count,
        13
    );
    bucket.data[173] = 1;
    assert!(ReceiptBucket::decode(&key(24), &bucket, &p.program).is_err());
    let mut token = RawAccount {
        owner: address::TOKEN_PROGRAM,
        lamports: 1,
        executable: false,
        data: vec![0; 165],
    };
    put(&mut token.data, 0, p.mint.as_ref());
    put(&mut token.data, 32, u.owner.as_ref());
    put(&mut token.data, 64, &u64::MAX.to_le_bytes());
    token.data[108] = 1;
    TokenAccount::decode(&token)
        .unwrap()
        .validate(&p.mint, &u.owner)
        .unwrap();
    token.data[108] = 2;
    assert!(
        TokenAccount::decode(&token)
            .unwrap()
            .validate(&p.mint, &u.owner)
            .is_err()
    );
}

#[test]
fn metadata_uri_matches_indexer_rules_and_preserves_public_queries() {
    let uri = "https://example.com/provider.json?v=2";
    assert_eq!(
        fossa_sdk::metadata::validate_uri(uri).unwrap().as_str(),
        uri
    );
    for uri in [
        "http://example.com/p.json",
        "https://127.0.0.1/p.json",
        "https://[::1]/p.json",
        "https://localhost/p.json",
        "https://provider.local/p.json",
        "https://example.com:8443/p.json",
        "https://user:password@example.com/p.json",
        "https://example.com/p.json#v2",
    ] {
        assert!(fossa_sdk::metadata::validate_uri(uri).is_err(), "{uri}");
    }
    assert_eq!(
        fossa_sdk::hosted::validate_api_origin("https://api.example.com/").unwrap(),
        "https://api.example.com"
    );
    for origin in [
        "https://user:password@api.example.com",
        "https://api.example.com?key=private",
        "https://api.example.com/path",
    ] {
        assert!(fossa_sdk::hosted::validate_api_origin(origin).is_err());
    }
}
