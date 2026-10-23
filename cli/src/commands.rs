use crate::{
    Error, Result,
    args::*,
    config::Config,
    runtime::{self, Runtime},
};
use fossa_sdk::{
    Keypair, PendingTransaction, Pubkey, Signer,
    accounts::*,
    address,
    amount::{self, Rates, Usage},
    instruction::{self, ModelKeys, OfferKeys, SettlementKeys, UserKeys},
    metadata::{Document, Kind},
    onboarding,
};
use serde_json::{Value, json};
use std::{io::Read, path::Path, time::Duration};

pub async fn run(cli: Cli) -> Result<Value> {
    preflight(&cli.command)?;
    if let Command::Metadata(MetadataCommand::Validate { kind, file }) = &cli.command {
        return Ok(document_view(&document(*kind, file)?));
    }
    let config = Config::load(&cli.options)?;
    if matches!(cli.command, Command::Onboard { .. }) {
        validate_onboard(&config)?;
    }
    match &cli.command {
        Command::Config(ConfigCommand::Show) => {
            return Ok(
                json!({"api_url":config.api_url,"rpc":if config.hosted_rpc { "hosted" } else if config.rpc_url.is_some() { "custom (URL redacted)" } else { "disabled" },"wallet":config.wallet.map(|w|w.to_string()),"keypair_configured":config.keypair.is_some(),"separate_payer":config.payer.is_some(),"journal_dir":config.journal,"program_id":config.program_id.to_string(),"market":config.market.to_string(),"usdc_mint":config.mint.to_string(),"mode":if cli.options.execute {"execute"} else {"plan"}}),
            );
        }
        Command::Metadata(MetadataCommand::Upload { kind, file }) => {
            let doc = document(*kind, file)?;
            return if cli.options.execute {
                Ok(
                    json!({"mode":"uploaded","upload":runtime::hosted(&config)?.upload_metadata(&doc).await?}),
                )
            } else {
                Ok(
                    json!({"mode":"plan","operation":"upload metadata","document":document_view(&doc),"reference":reference_view(&hosted_reference(&config,&doc)?)}),
                )
            };
        }
        Command::Image(ImageCommand::Upload { file }) => {
            let (mime, bytes) = image(file)?;
            return if cli.options.execute {
                Ok(
                    json!({"mode":"uploaded","upload":runtime::hosted(&config)?.upload_image(mime,&bytes).await?}),
                )
            } else {
                Ok(
                    json!({"mode":"plan","operation":"upload image","content_type":mime,"bytes":bytes.len(),"sha256":fossa_sdk::metadata::sha256_hex(&bytes)}),
                )
            };
        }
        Command::Transaction(command) => return transaction(&config, &cli.options, command).await,
        _ => (),
    }
    let rt = Runtime::connect(config, cli.options).await?;
    match cli.command {
        Command::Provider(command) => provider(&rt, command).await,
        Command::Model(command) => model(&rt, command).await,
        Command::Offer(command) => offer(&rt, command).await,
        Command::Bond(command) => bond(&rt, command).await,
        Command::Account(AccountCommand::Show { address, kind }) => {
            account(&rt, address, kind).await
        }
        Command::Onboard {
            allow_approval,
            admin,
        } => onboard(&rt, allow_approval, admin.as_deref()).await,
        Command::User(command) => user(&rt, command).await,
        Command::Booking(command) => booking(&rt, command).await,
        _ => unreachable!(),
    }
}
fn preflight(command: &Command) -> Result<()> {
    match command {
        Command::Metadata(
            MetadataCommand::Validate { kind, file } | MetadataCommand::Upload { kind, file },
        ) => {
            document(*kind, file)?;
        }
        Command::Image(ImageCommand::Upload { file }) => {
            image(file)?;
        }
        Command::Provider(ProviderCommand::Register(args)) => {
            document(MetadataKind::Provider, &args.metadata_file)?;
            validate_uri(args.metadata_uri.as_deref())?;
        }
        Command::Provider(ProviderCommand::Update(args)) => {
            Document::parse(
                Kind::Provider,
                serde_json::to_vec(
                    &json!({"name":args.name.as_deref().unwrap_or("Profile"),"endpoint":"https://example.invalid","image":args.image_url}),
                )?,
            )?;
            if let Some(path) = &args.metadata_file {
                document(MetadataKind::Provider, path)?;
            }
            if let Some(path) = &args.image_file {
                image(path)?;
            }
            validate_uri(args.metadata_uri.as_deref())?;
        }
        Command::Model(ModelCommand::UpdateMetadata {
            model,
            document: args,
        }) => {
            model_document(&model.model, &args.metadata_file)?;
            validate_uri(args.metadata_uri.as_deref())?;
        }
        Command::Offer(OfferCommand::Update { patch, .. }) => {
            if matches!((patch.input_price,patch.cached_price),(Some(input),Some(cached)) if cached>input)
            {
                return Err(Error::Input(
                    "cached price must not exceed input price".into(),
                ));
            }
        }
        Command::Booking(BookingCommand::Create(args)) => {
            book_request(args).validate()?;
        }
        _ => (),
    }
    Ok(())
}
fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Input(format!("file exceeds {limit} bytes")));
    }
    Ok(bytes)
}
fn document(kind: MetadataKind, path: &Path) -> Result<Document> {
    Ok(Document::parse(
        kind.into(),
        read_limited(path, fossa_sdk::metadata::MAX_BYTES)?,
    )?)
}
fn model_document(id: &str, path: &Path) -> Result<Document> {
    Ok(Document::model(
        id,
        read_limited(path, fossa_sdk::metadata::MAX_BYTES)?,
    )?)
}
fn image(path: &Path) -> Result<(&'static str, Vec<u8>)> {
    let bytes = read_limited(path, 2 * 1024 * 1024)?;
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        return Err(Error::Input("expected PNG, JPEG or WebP image".into()));
    };
    Ok((mime, bytes))
}
fn validate_uri(uri: Option<&str>) -> Result<()> {
    if let Some(uri) = uri {
        fossa_sdk::metadata::validate_uri(uri)?;
    }
    Ok(())
}

fn document_view(doc: &Document) -> Value {
    json!({"sha256":runtime::hex(&doc.sha256()),"bytes":doc.bytes().len(),"document":doc.value()})
}
fn reference_view(reference: &DocumentRef) -> Value {
    json!({"sha256":runtime::hex(&reference.sha256),"uri":reference.uri})
}
fn hosted_reference(config: &Config, doc: &Document) -> Result<DocumentRef> {
    Ok(doc.reference(format!(
        "{}/v1/market/metadata/{}",
        config.api_url.trim_end_matches('/'),
        runtime::hex(&doc.sha256())
    ))?)
}
async fn publish(rt: &Runtime, doc: &Document, uri: Option<&str>) -> Result<DocumentRef> {
    if let Some(uri) = uri {
        let reference = doc.reference(uri)?;
        if rt.options.execute {
            Document::fetch(doc.kind(), &reference).await?;
        }
        return Ok(reference);
    }
    if rt.options.execute {
        rt.ensure_no_pending()?;
        Ok(runtime::hosted(&rt.config)?
            .upload_metadata(doc)
            .await?
            .reference()?)
    } else {
        hosted_reference(&rt.config, doc)
    }
}
async fn provider(rt: &Runtime, command: ProviderCommand) -> Result<Value> {
    let wallet = rt.wallet()?;
    let address = address::provider(&rt.protocol.program, &rt.protocol.market, &wallet).0;
    if matches!(command, ProviderCommand::Show) {
        return Ok(
            json!({"address":address.to_string(),"provider":rt.rpc.account::<Provider>(address,rt.protocol.program).await?}),
        );
    }
    let state = rt
        .rpc
        .account::<Provider>(address, rt.protocol.program)
        .await?;
    match command {
        ProviderCommand::Register(args) => {
            if state.is_some() {
                return Err(Error::Input(
                    "provider already registered; use provider update".into(),
                ));
            }
            let doc = document(MetadataKind::Provider, &args.metadata_file)?;
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            let ix = rt
                .protocol
                .initialize_provider(wallet, rt.payer()?, &reference)?;
            rt.transact("register provider",vec![ix],&[],json!({"provider":address.to_string(),"metadata":reference_view(&reference),"approval":"requires market admin"})).await
        }
        ProviderCommand::Update(args) => {
            let state = state
                .ok_or_else(|| Error::Input("provider missing; use provider register".into()))?;
            let doc = if let Some(path) = &args.metadata_file {
                document(MetadataKind::Provider, path)?
            } else {
                let current = Document::fetch(Kind::Provider, &state.document).await?;
                let image_data = args.image_file.as_deref().map(image).transpose()?;
                let image_uri = image_data
                    .as_ref()
                    .map(|(_, bytes)| {
                        format!(
                            "{}/v1/providers/images/{}",
                            rt.config.api_url,
                            fossa_sdk::metadata::sha256_hex(bytes)
                        )
                    })
                    .or_else(|| args.image_url.clone());
                let document = patch_profile(&current, args.name.as_deref(), image_uri.as_deref())?;
                if rt.options.execute
                    && let Some((mime, bytes)) = &image_data
                {
                    rt.ensure_no_pending()?;
                    runtime::hosted(&rt.config)?
                        .upload_image(mime, bytes)
                        .await?;
                }
                document
            };
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            let ix = rt.protocol.update_provider_metadata(wallet, &reference)?;
            rt.transact("update provider metadata",vec![ix],&[],json!({"provider":address.to_string(),"previous":reference_view(&state.document),"metadata":reference_view(&reference),"document":doc.value(),"image_upload":args.image_file.is_some()})).await
        }
        ProviderCommand::Show => unreachable!(),
    }
}
pub fn patch_profile(
    current: &Document,
    name: Option<&str>,
    image: Option<&str>,
) -> Result<Document> {
    let mut value = current.value().clone();
    if let Some(name) = name {
        value["name"] = name.into();
    }
    if let Some(image) = image {
        value["image"] = image.into();
    }
    Ok(Document::parse(
        Kind::Provider,
        serde_json::to_vec(&value)?,
    )?)
}
fn model_address(rt: &Runtime, id: &str) -> Result<Pubkey> {
    if id.is_empty() {
        return Err(Error::Input("model identifier must not be empty".into()));
    }
    Ok(address::model(
        &rt.protocol.program,
        &rt.protocol.market,
        &address::model_id(id),
    )
    .0)
}
async fn model(rt: &Runtime, command: ModelCommand) -> Result<Value> {
    match command {
        ModelCommand::Show(args) => {
            let address = model_address(rt, &args.model)?;
            let (model, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
            let metadata = rt
                .rpc
                .account::<ModelMetadata>(
                    address::model_metadata(&rt.protocol.program, &address).0,
                    rt.protocol.program,
                )
                .await?;
            Ok(
                json!({"address":address.to_string(),"model":model,"metadata":metadata,"terms":terms,"capacity":capacity}),
            )
        }
        ModelCommand::UpdateMetadata {
            model,
            document: args,
        } => {
            let address = model_address(rt, &model.model)?;
            let metadata = rt
                .rpc
                .require_account::<ModelMetadata>(
                    address::model_metadata(&rt.protocol.program, &address).0,
                    rt.protocol.program,
                )
                .await?;
            if metadata.market != rt.protocol.market
                || metadata.model != address
                || metadata.authority != rt.wallet()?
            {
                return Err(Error::Input(
                    "model metadata authority or binding mismatch".into(),
                ));
            }
            let doc = model_document(&model.model, &args.metadata_file)?;
            let reference = publish(rt, &doc, args.metadata_uri.as_deref()).await?;
            rt.transact(
                "update model metadata",
                vec![
                    rt.protocol
                        .update_model_metadata(address, rt.wallet()?, &reference)?,
                ],
                &[],
                json!({"model":model.model,"metadata":reference_view(&reference)}),
            )
            .await
        }
    }
}
pub fn patch_terms(mut terms: OfferTerms, patch: &OfferPatch) -> Result<OfferTerms> {
    if let Some(n) = patch.input_price {
        terms.rates.input = n;
    }
    if let Some(n) = patch.cached_price {
        terms.rates.cached = n;
    }
    if let Some(n) = patch.output_price {
        terms.rates.output = n;
    }
    if let Some(n) = patch.capacity {
        terms.capacity = n;
    }
    if let Some(n) = patch.speed_tps {
        terms.speed_tps = n;
    }
    if let Some(v) = patch.active {
        terms.active = v;
    }
    if let Some(v) = patch.private {
        terms.private = v;
    }
    terms.validate()?;
    Ok(terms)
}
async fn selected_offer(rt: &Runtime, args: &OfferArg) -> Result<(OfferKeys, Offer, Capacity)> {
    offer_at(rt, model_address(rt, &args.model.model)?, args.slot).await
}
async fn offer_at(
    rt: &Runtime,
    address: Pubkey,
    slot: Option<u32>,
) -> Result<(OfferKeys, Offer, Capacity)> {
    let (model, terms, capacities) = rt.rpc.model_books(&rt.protocol, address).await?;
    let wallet = rt.wallet()?;
    let slot = if let Some(n) = slot {
        n
    } else {
        let matches = terms
            .entries
            .iter()
            .enumerate()
            .filter(|(_, offer)| offer.provider == wallet)
            .map(|(i, _)| i as u32)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(Error::Input(
                "select --slot: wallet must have exactly one offer to infer slot".into(),
            ));
        }
        matches[0]
    };
    let offer = terms
        .entries
        .get(slot as usize)
        .ok_or_else(|| Error::Input("offer slot out of range".into()))?
        .clone();
    if offer.provider != wallet {
        return Err(Error::Input("offer belongs to another provider".into()));
    }
    let keys = OfferKeys {
        model: ModelKeys::from_account(address, &model),
        provider: wallet,
        payout: offer.payout,
        bond_vault: offer.bond_vault,
        slot,
    };
    Ok((keys, offer, capacities.entries[slot as usize]))
}
async fn offer(rt: &Runtime, command: OfferCommand) -> Result<Value> {
    match command {
        OfferCommand::List(args) => {
            let address = model_address(rt, &args.model)?;
            let (_, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
            Ok(
                json!({"model":args.model,"address":address.to_string(),"rate_unit":"micro-USDC per 1,000,000 tokens","offers":terms.entries,"capacity":capacity.entries}),
            )
        }
        OfferCommand::Update { offer, patch } => {
            let (keys, current, _) = selected_offer(rt, &offer).await?;
            let next = patch_terms(current.terms, &patch)?;
            rt.transact("update offer",vec![rt.protocol.update_offer(keys,next)?],&[],json!({"model":offer.model.model,"slot":keys.slot,"previous":current.terms,"next":next,"rate_unit":"micro-USDC per 1,000,000 tokens"})).await
        }
    }
}
async fn bond(rt: &Runtime, command: BondCommand) -> Result<Value> {
    match command {
        BondCommand::Fund {
            offer,
            amount,
            source,
        } => {
            let (keys, _, _) = selected_offer(rt, &offer).await?;
            let source = source
                .unwrap_or_else(|| address::associated_token(&rt.protocol.mint, &keys.provider));
            validate_token(rt, source, keys.provider).await?;
            rt.transact("fund bond",vec![rt.protocol.fund_bond(keys,source,amount)?],&[],json!({"amount_atomic":amount.to_string(),"amount_usdc":amount::format_usdc(amount),"model":offer.model.model,"slot":keys.slot})).await
        }
        BondCommand::Request { offer, amount } => {
            let (keys, _, _) = selected_offer(rt, &offer).await?;
            let request = Keypair::new();
            let request_address = request.pubkey();
            let payer = rt.payer()?;
            let ix = vec![
                rt.protocol.create_account(
                    payer,
                    request.pubkey(),
                    rt.rpc.rent(UNBOND_LEN).await?,
                    UNBOND_LEN,
                )?,
                rt.protocol
                    .request_unbond(keys, request.pubkey(), payer, amount)?,
            ];
            rt.transact("request unbond",ix,&[request],json!({"request":request_address.to_string(),"amount_atomic":amount.to_string(),"model":offer.model.model,"slot":keys.slot})).await
        }
        BondCommand::Execute { request } => {
            let state = rt
                .rpc
                .require_account::<Unbond>(request, rt.protocol.program)
                .await?;
            if state.market != rt.protocol.market
                || state.provider != rt.wallet()?
                || state.executed
            {
                return Err(Error::Input(
                    "unbond request binding or executed state invalid".into(),
                ));
            }
            let (keys, _, _) = offer_at(rt, state.model, Some(state.slot)).await?;
            rt.transact("execute unbond",vec![rt.protocol.execute_unbond(keys,request)],&[],json!({"request":request.to_string(),"amount_atomic":state.amount.to_string(),"ready":state.ready})).await
        }
    }
}
async fn validate_token(rt: &Runtime, account: Pubkey, owner: Pubkey) -> Result<()> {
    let raw = rt
        .rpc
        .raw_account(account)
        .await?
        .ok_or(fossa_sdk::Error::MissingAccount(account))?;
    TokenAccount::decode(&raw)?.validate(&rt.protocol.mint, &owner)?;
    Ok(())
}
async fn account(rt: &Runtime, address: Pubkey, kind: AccountKind) -> Result<Value> {
    macro_rules! typed {
        ($t:ty) => {
            serde_json::to_value(
                rt.rpc
                    .require_account::<$t>(address, rt.protocol.program)
                    .await?,
            )?
        };
    }
    let state = match kind {
        AccountKind::Market => typed!(Market),
        AccountKind::Provider => typed!(Provider),
        AccountKind::Model => typed!(Model),
        AccountKind::ModelMetadata => typed!(ModelMetadata),
        AccountKind::User => typed!(User),
        AccountKind::Terms => typed!(TermsBook),
        AccountKind::Capacity => typed!(CapacityBook),
        AccountKind::Booking => typed!(Booking),
        AccountKind::Unbond => typed!(Unbond),
        AccountKind::ReceiptBucket => typed!(ReceiptBucket),
        AccountKind::Token => serde_json::to_value(TokenAccount::decode(
            &rt.rpc
                .raw_account(address)
                .await?
                .ok_or(fossa_sdk::Error::MissingAccount(address))?,
        )?)?,
    };
    Ok(json!({"address":address.to_string(),"account":state}))
}
async fn transaction(
    config: &Config,
    options: &Options,
    command: &TransactionCommand,
) -> Result<Value> {
    let (path, resume) = match command {
        TransactionCommand::Status { file } => (file, false),
        TransactionCommand::Resume { file } => (file, true),
    };
    let _journal_lock = runtime::JournalLock::acquire(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let pending = PendingTransaction::load(path)?;
    let rpc = runtime::rpc(config)?;
    let signature = pending.validate()?;
    if resume && options.execute {
        rpc.submit(&pending)
            .await
            .map_err(|e| runtime::pending_error(&pending, path, e))?;
        let status = rpc
            .confirm(&pending, Duration::from_secs(options.confirmation_seconds))
            .await
            .map_err(|e| runtime::pending_error(&pending, path, e))?;
        let archive = runtime::archive_pending(path, &pending)?;
        return Ok(
            json!({"mode":"executed","signature":pending.signature,"status":status,"journal":archive}),
        );
    }
    let mut status = rpc.signature_status(&signature).await?;
    let finalized_height = if status.is_none() {
        Some(
            rpc.clone()
                .with_commitment(fossa_sdk::rpc::Commitment::Finalized)
                .block_height()
                .await?,
        )
    } else {
        None
    };
    let expired = finalized_height.is_some_and(|n| n > pending.last_valid_block_height);
    if expired {
        status = rpc.signature_status(&signature).await?;
    }
    Ok(
        json!({"mode":if resume {"plan"} else {"inspection"},"signature":pending.signature,"status":status,"expired_without_observed_status":expired&&status.is_none(),"last_valid_block_height":pending.last_valid_block_height,"journal":path,"next":if resume {"--execute reconciles then resends only original bytes"} else {"use transaction resume --file PATH --execute; preserve unresolved journals"}}),
    )
}
async fn user(rt: &Runtime, command: UserCommand) -> Result<Value> {
    if let UserCommand::Show { user } = command {
        return account(rt, user, AccountKind::User).await;
    }
    let wallet = rt.wallet()?;
    if let UserCommand::Init = command {
        let key = Keypair::new();
        let address = key.pubkey();
        let authority = address::vault(&rt.protocol.program, &address).0;
        let vault = address::associated_token(&rt.protocol.mint, &authority);
        let payer = rt.payer()?;
        let ix = vec![
            rt.protocol
                .create_account(payer, address, rt.rpc.rent(USER_LEN).await?, USER_LEN)?,
            instruction::create_associated_token(payer, authority, rt.protocol.mint),
            rt.protocol.initialize_user(UserKeys {
                user: address,
                owner: wallet,
                vault,
            }),
        ];
        return rt
            .transact(
                "initialize user",
                ix,
                &[key],
                json!({"user":address.to_string(),"vault":vault.to_string()}),
            )
            .await;
    }
    let (address, amount, token, deposit) = match command {
        UserCommand::Deposit {
            user,
            amount,
            source,
        } => (user, amount, source, true),
        UserCommand::Withdraw {
            user,
            amount,
            destination,
        } => (user, amount, destination, false),
        _ => unreachable!(),
    };
    let state = rt
        .rpc
        .require_account::<User>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market || state.owner != wallet {
        return Err(Error::Input(
            "user account authority or market mismatch".into(),
        ));
    }
    let default_token = token.is_none();
    let token = token.unwrap_or_else(|| address::associated_token(&rt.protocol.mint, &wallet));
    let keys = UserKeys::from_account(address, &state);
    let ix = if deposit {
        validate_token(rt, token, wallet).await?;
        vec![rt.protocol.deposit(keys, token, amount)?]
    } else {
        let mut instructions = Vec::with_capacity(2);
        if default_token {
            instructions.push(instruction::create_associated_token(
                rt.payer()?,
                wallet,
                rt.protocol.mint,
            ));
        }
        instructions.push(rt.protocol.withdraw(keys, token, amount)?);
        instructions
    };
    rt.transact(if deposit {"deposit user funds"} else {"withdraw user funds"},ix,&[],json!({"user":address.to_string(),"amount_atomic":amount.to_string(),"amount_usdc":amount::format_usdc(amount)})).await
}
async fn booking(rt: &Runtime, command: BookingCommand) -> Result<Value> {
    if let BookingCommand::Create(args) = command {
        return book(rt, args).await;
    }
    let address = match command {
        BookingCommand::Show { booking }
        | BookingCommand::Claim { booking, .. }
        | BookingCommand::Release { booking }
        | BookingCommand::Expire { booking }
        | BookingCommand::Finalize { booking } => booking,
        BookingCommand::Create(_) => unreachable!(),
    };
    if matches!(command, BookingCommand::Show { .. }) {
        return account(rt, address, AccountKind::Booking).await;
    }
    let state = rt
        .rpc
        .require_account::<Booking>(address, rt.protocol.program)
        .await?;
    if state.market != rt.protocol.market {
        return Err(Error::Input("booking market mismatch".into()));
    }
    let model = rt
        .rpc
        .require_account::<Model>(state.model, rt.protocol.program)
        .await?;
    let model = ModelKeys::from_account(state.model, &model);
    let user = rt
        .rpc
        .require_account::<User>(state.user, rt.protocol.program)
        .await?;
    if user.market != rt.protocol.market {
        return Err(Error::Input("booking user market mismatch".into()));
    }
    let keys = SettlementKeys::from_booking(
        address,
        &state,
        model,
        UserKeys::from_account(state.user, &user),
    )?;
    let generation = (state.generation > 0).then_some(state.generation);
    let (label, ix) = match command {
        BookingCommand::Claim {
            uncached,
            cached,
            output,
            ..
        } => {
            if state.provider != rt.wallet()? {
                return Err(Error::Input("booking belongs to another provider".into()));
            }
            if generation.is_some() {
                return Err(Error::Input("reusable booking claims require receipt publication; use the SDK receipt workflow".into()));
            }
            if u64::from(uncached) + u64::from(cached) > u64::from(state.max_input)
                || output > state.max_output
            {
                return Err(Error::Input("usage exceeds booking token caps".into()));
            }
            (
                "claim booking",
                rt.protocol.claim(
                    keys,
                    Usage {
                        uncached,
                        cached,
                        output,
                    },
                    None,
                )?,
            )
        }
        BookingCommand::Release { .. } => {
            if state.provider != rt.wallet()? {
                return Err(Error::Input("booking belongs to another provider".into()));
            }
            ("release booking", rt.protocol.release(keys, generation)?)
        }
        BookingCommand::Expire { .. } => ("expire booking", rt.protocol.expire(keys, generation)?),
        BookingCommand::Finalize { .. } => {
            ("finalize booking", rt.protocol.finalize(model, address))
        }
        _ => unreachable!(),
    };
    rt.transact(
        label,
        vec![ix],
        &[],
        json!({"booking":address.to_string(),"generation":state.generation.to_string()}),
    )
    .await
}
async fn onboard(rt: &Runtime, allow_approval: bool, admin_path: Option<&Path>) -> Result<Value> {
    let cfg = &rt.config.file.provider;
    let file = cfg
        .metadata_file
        .as_ref()
        .ok_or_else(|| Error::Input("onboard requires provider.metadata_file".into()))?;
    let doc = document(MetadataKind::Provider, &rt.config.base.join(file))?;
    validate_uri(cfg.metadata_uri.as_deref())?;
    let reference = match &cfg.metadata_uri {
        Some(uri) => doc.reference(uri)?,
        None => hosted_reference(&rt.config, &doc)?,
    };
    let mut documents = vec![(doc, cfg.metadata_uri.as_deref())];
    let mut models = Vec::new();
    for model in &cfg.models {
        let document = model_document(&model.id, &rt.config.base.join(&model.metadata_file))?;
        validate_uri(model.metadata_uri.as_deref())?;
        let reference = match &model.metadata_uri {
            Some(uri) => document.reference(uri)?,
            None => hosted_reference(&rt.config, &document)?,
        };
        let terms = OfferTerms {
            rates: Rates {
                input: model.input_rate,
                cached: model.cached_rate,
                output: model.output_rate,
            },
            speed_tps: model
                .speed_tps
                .or(cfg.speed_tps)
                .ok_or_else(|| Error::Input("configure speed_tps".into()))?,
            capacity: model
                .capacity
                .or(cfg.capacity)
                .ok_or_else(|| Error::Input("configure capacity".into()))?,
            private: model.private.or(cfg.private).unwrap_or(false),
            active: model.active.or(cfg.active).unwrap_or(true),
        };
        terms.validate()?;
        let bond_target = amount::parse_atomic(
            model
                .bond_target_atomic
                .as_deref()
                .or(cfg.bond_target_atomic.as_deref())
                .unwrap_or("0"),
        )?;
        models.push(onboarding::ModelConfig {
            id: model.id.clone(),
            document: reference,
            terms,
            bond_target,
            slot: model.slot,
        });
        documents.push((document, model.metadata_uri.as_deref()));
    }
    let admin = admin_path
        .map(fossa_sdk::transaction::read_keypair)
        .transpose()?;
    let config = onboarding::OnboardConfig {
        provider: rt.wallet()?,
        payer: rt.payer()?,
        admin: admin.as_ref().map(Signer::pubkey),
        document: reference,
        book_limit: cfg.model_book_limit.unwrap_or(16),
        models,
    };
    let mut plan = onboarding::plan(&rt.rpc, &rt.protocol, &config).await?;
    if !allow_approval
        && plan.steps.iter().any(|step| {
            step.instructions
                .iter()
                .any(|ix| ix.program_id == rt.protocol.program && ix.data.first() == Some(&20))
        })
    {
        plan.blockers.push("provider approval requires explicit onboard --allow-approval and market admin signature".into());
    }
    let view = json!({"mode":"plan","operation":"onboard","blockers":plan.blockers,"steps":plan.steps.iter().map(|s|json!({"operation":s.label,"instructions":runtime::instruction_view(&s.instructions)})).collect::<Vec<_>>(),"metadata":documents.iter().map(|(d,_)|document_view(d)).collect::<Vec<_>>()});
    if !rt.options.execute {
        return Ok(view);
    }
    if !plan.blockers.is_empty() {
        return Err(Error::Input(format!(
            "onboarding blocked: {}",
            plan.blockers.join("; ")
        )));
    }
    rt.ensure_no_pending()?;
    for (doc, uri) in &documents {
        publish(rt, doc, *uri).await?;
    }
    let mut completed = Vec::new();
    for mut step in plan.steps {
        if let Some(path) = admin_path {
            step.signers
                .push(fossa_sdk::transaction::read_keypair(path)?);
        }
        completed.push(
            rt.transact(
                &step.label,
                step.instructions,
                &step.signers,
                json!({"onboarding":true}),
            )
            .await?,
        );
    }
    Ok(json!({"mode":"executed","operation":"onboard","steps":completed}))
}

fn book_request(args: &BookArgs) -> instruction::BookRequest {
    instruction::BookRequest {
        model_id: address::model_id(&args.model.model),
        rate_caps: Rates {
            input: args.input_price_cap,
            cached: args.cached_price_cap,
            output: args.output_price_cap,
        },
        min_speed: args.min_speed,
        max_input: args.max_input,
        max_output: args.max_output,
        private: args.private,
        max_spend: args.max_spend,
        estimated_input: args.estimated_input.unwrap_or(args.max_input),
        estimated_cached: args.estimated_cached,
        estimated_output: args.estimated_output.unwrap_or(args.max_output),
        cache_provider: args.cache_provider.unwrap_or_default(),
    }
}
async fn book(rt: &Runtime, args: BookArgs) -> Result<Value> {
    let user = rt
        .rpc
        .require_account::<User>(args.user, rt.protocol.program)
        .await?;
    if user.market != rt.protocol.market || user.owner != rt.wallet()? {
        return Err(Error::Input(
            "user account authority or market mismatch".into(),
        ));
    }
    let address = model_address(rt, &args.model.model)?;
    let (model, terms, capacity) = rt.rpc.model_books(&rt.protocol, address).await?;
    let request = book_request(&args);
    let selected = request
        .select(&terms, &capacity, user.free)?
        .ok_or_else(|| {
            Error::Input(
                "no eligible offer fits price, speed, privacy, capacity, bond and balance limits"
                    .into(),
            )
        })?;
    let key = Keypair::new();
    let booking = key.pubkey();
    let payer = rt.payer()?;
    let ix = vec![
        fossa_sdk::transaction::compute_unit_limit(
            12_000 + 200 * terms.entries.len().min(4096) as u32,
        ),
        rt.protocol
            .create_account(payer, booking, rt.rpc.rent(BOOKING_LEN).await?, BOOKING_LEN)?,
        rt.protocol.book(
            ModelKeys::from_account(address, &model),
            UserKeys::from_account(args.user, &user),
            booking,
            payer,
            &request,
            instruction::BookingMode::OneShot,
        )?,
    ];
    rt.transact("create booking",ix,&[key],json!({"booking":booking.to_string(),"user":args.user.to_string(),"candidate_slot":selected.slot,"reserve_atomic":selected.reserve.to_string(),"selection":"on-chain selection uses execution-time state"})).await
}
fn validate_onboard(config: &Config) -> Result<()> {
    let provider = &config.file.provider;
    let path = provider
        .metadata_file
        .as_ref()
        .ok_or_else(|| Error::Input("onboard requires provider.metadata_file".into()))?;
    document(MetadataKind::Provider, &config.base.join(path))?;
    validate_uri(provider.metadata_uri.as_deref())?;
    instruction::book_sizes(provider.model_book_limit.unwrap_or(16))?;
    for model in &provider.models {
        model_document(&model.id, &config.base.join(&model.metadata_file))?;
        validate_uri(model.metadata_uri.as_deref())?;
        OfferTerms {
            rates: Rates {
                input: model.input_rate,
                cached: model.cached_rate,
                output: model.output_rate,
            },
            speed_tps: model
                .speed_tps
                .or(provider.speed_tps)
                .ok_or_else(|| Error::Input("configure speed_tps".into()))?,
            capacity: model
                .capacity
                .or(provider.capacity)
                .ok_or_else(|| Error::Input("configure capacity".into()))?,
            private: model.private.or(provider.private).unwrap_or(false),
            active: model.active.or(provider.active).unwrap_or(true),
        }
        .validate()?;
        amount::parse_atomic(
            model
                .bond_target_atomic
                .as_deref()
                .or(provider.bond_target_atomic.as_deref())
                .unwrap_or("0"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn price_patch_preserves_capacity_privacy_activation_speed_and_other_rates() {
        let old = OfferTerms {
            rates: Rates {
                input: 1_000_000,
                cached: 500_000,
                output: 2_000_000,
            },
            speed_tps: 300,
            private: true,
            active: false,
            capacity: 7,
        };
        let next = patch_terms(
            old,
            &OfferPatch {
                output_price: Some(3_123_456),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            next,
            OfferTerms {
                rates: Rates {
                    output: 3_123_456,
                    ..old.rates
                },
                ..old
            }
        );
        assert!(
            patch_terms(
                old,
                &OfferPatch {
                    input_price: Some(1),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn profile_patch_preserves_extensions_changes_exact_hash_and_new_reference() {
        let bytes=br#"{ "name":"Old", "endpoint":"https://provider.example/infer", "image":"https://provider.example/old.png", "extension":{"a":[1,true,"opaque"]} }"#;
        let current = Document::parse(Kind::Provider, bytes.to_vec()).unwrap();
        let old = current.reference("https://example.com/old.json").unwrap();
        current.verify(&old).unwrap();
        let next =
            patch_profile(&current, Some("New"), Some("https://example.com/new.png")).unwrap();
        assert_eq!(next.value()["extension"], current.value()["extension"]);
        assert_eq!(next.value()["endpoint"], current.value()["endpoint"]);
        assert_eq!(next.value()["name"], "New");
        assert_eq!(next.value()["image"], "https://example.com/new.png");
        assert!(next.verify(&old).is_err());
        assert_eq!(
            next.reference("https://example.com/new.json")
                .unwrap()
                .sha256,
            next.sha256()
        );
    }
    #[test]
    fn invalid_profile_patch_cannot_reach_upload() {
        let current = Document::parse(
            Kind::Provider,
            br#"{"name":"Old","endpoint":"https://example.com/infer"}"#.to_vec(),
        )
        .unwrap();
        assert!(patch_profile(&current, Some(" "), None).is_err());
        assert!(patch_profile(&current, None, Some("http://example.com/logo.png")).is_err());
    }
}
