use crate::*;

pub(crate) fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    install_sigint_handler();
    CHECK_INTERRUPTED.store(false, Ordering::SeqCst);
    let command = parse_check_command_args(args)?;
    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, &command.config_path)?;
    let options = parse_check_options(&config, &command.option_args)?;
    fail_on_mixed_canon_changes(root)?;
    // Apply the staged Git snapshot as an in-place index view: unstaged and
    // untracked worktree changes are preserved away, so the evaluator sees the
    // index contents at the real project root. This creates no copied
    // repository, copied tree, or copied snapshot directory. File visibility is
    // enforced by app-server permissions, not by copying the repository to a
    // filtered view.
    let _staged_view = StagedWorktreeView::apply(root)?;
    let mut runner = LazyAppServerRunner::new(check_config_loads_plugins(&config), &config.agent);
    let mut diagnostic_log = DiagnosticLogWriter::create_with_cache(root, &mut repo_cache)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut result_output: &mut dyn Write = &mut stdout;
    // `run_check_with_runner` calls `write_and_flush_result_output` after each
    // selected expectation, so stdout observers receive each JSONL result before
    // the next expectation starts.
    let records_result = run_check_with_runner(
        root,
        root,
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        Some(&mut result_output),
    );
    result_output
        .flush()
        .map_err(|err| format!("failed to flush check result to stdout: {}", err))?;
    runner.drain_token_usage_updates();
    print_token_usage_summary(runner.token_usage(), check_interrupted());
    let records = records_result?;
    if records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err("canon check failed".to_string())
    }
}

pub(crate) fn print_token_usage_summary(usage: Option<TokenUsage>, force: bool) {
    if let Some(usage) = usage {
        eprintln!("{}", render_token_usage_summary(usage));
    } else if force {
        eprintln!("{}", render_token_usage_summary(TokenUsage::default()));
    }
}

pub(crate) fn install_sigint_handler() {
    SIGNAL_HANDLER_INIT.call_once(|| {
        #[cfg(unix)]
        unsafe {
            const SIGHUP: i32 = 1;
            const SIGINT: i32 = 2;
            const SIGTERM: i32 = 15;
            let _ = signal(SIGHUP, handle_sigint);
            let _ = signal(SIGINT, handle_sigint);
            let _ = signal(SIGTERM, handle_sigint);
        }
    });
}

pub(crate) fn check_interrupted() -> bool {
    CHECK_INTERRUPTED.load(Ordering::SeqCst)
}

pub(crate) fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.iter().any(|arg| arg.to_str() == Some("--fail-fast")) {
        return Err("canon gate does not accept --fail-fast".to_string());
    }
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".to_string());
    }
    let mut repo_cache = RepoInspectionCache::new();
    let config = repo_cache.load_check_config(root, Path::new(CHECK_PATH))?;
    let selected = select_expectations(&config, args)?;
    fail_on_mixed_canon_changes(root)?;

    // From this point on, `canon gate` is a cache-only decision. The only gate
    // failures after command/config/staged-change preflight are missing cache
    // records and new cached failures that were not already failing at HEAD.
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    let mut missing = Vec::new();
    let mut failing = Vec::new();
    for expectation in &selected {
        match reusable_history_record_for_source(
            root,
            &config.agent,
            expectation,
            ScopeHashSource::Index,
            &mut history_cache,
            &mut scope_hash_cache,
        )? {
            Some(record) if record.passed() => {}
            Some(record)
                if !record.passed()
                    && has_reusable_head_failure(
                        root,
                        &config.agent,
                        expectation,
                        &mut history_cache,
                        &mut scope_hash_cache,
                    )? => {}
            Some(record) => failing.push(record),
            None => missing.push(expectation.number),
        }
    }

    if missing.is_empty() && failing.is_empty() {
        return Ok(());
    }
    if !missing.is_empty() {
        eprintln!(
            "canon gate: missing cached answers for expectations: {}",
            join_numbers(&missing)
        );
        eprintln!("canon gate: run `canon check` before committing");
    }
    if !failing.is_empty() {
        eprintln!("canon gate: cached failing expectation results:");
        for record in &failing {
            eprint!("{}", render_check_log_record(record));
        }
    }
    Err("canon gate failed".to_string())
}

pub(crate) fn has_reusable_head_failure(
    root: &Path,
    agent: &AgentConfig,
    expectation: &SelectedExpectation,
    history_cache: &mut HistoryCache,
    scope_hash_cache: &mut ScopeHashCache,
) -> Result<bool, String> {
    Ok(matches!(
        reusable_history_record_for_source(
            root,
            agent,
            expectation,
            ScopeHashSource::Head,
            history_cache,
            scope_hash_cache
        )?,
        Some(record) if !record.passed()
    ))
}

pub(crate) fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
    let mut config_path = None;
    let mut option_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = arg_to_string(&args[index])?;
        if arg == "--config" || arg == "-c" {
            if config_path.is_some() {
                return Err("duplicate --config".to_string());
            }
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{} requires a path", arg))?;
            let value = arg_to_string(value)?;
            config_path = Some(normalize_check_config_path(&value)?);
        } else if let Some(value) = arg.strip_prefix("--config=") {
            if config_path.is_some() {
                return Err("duplicate --config".to_string());
            }
            if value.is_empty() {
                return Err("--config requires a path".to_string());
            }
            config_path = Some(normalize_check_config_path(value)?);
        } else {
            option_args.push(args[index].clone());
        }
        index += 1;
    }
    Ok(CheckCommandArgs {
        config_path: config_path.unwrap_or_else(|| PathBuf::from(CHECK_PATH)),
        option_args,
    })
}

pub(crate) fn normalize_check_config_path(value: &str) -> Result<PathBuf, String> {
    let normalized = normalize_repo_path(value).map_err(|err| format!("--config path: {}", err))?;
    if normalized == "." {
        return Err("--config path must name a file".to_string());
    }
    Ok(PathBuf::from(normalized))
}

pub(crate) fn parse_check_config_content(
    config_path: &Path,
    content: &str,
) -> Result<CheckConfig, String> {
    let config: CheckConfig = serde_yaml::from_str(&content)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))?;
    validate_check_config(&config)?;
    Ok(config)
}

pub(crate) fn parse_check_options(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<CheckOptions, String> {
    let mut fail_fast = false;
    let mut ignore_cache = false;
    let mut numbers = Vec::new();
    for arg in args {
        if arg.to_str() == Some("--fail-fast") {
            if fail_fast {
                return Err("duplicate --fail-fast".to_string());
            }
            fail_fast = true;
        } else if arg.to_str() == Some("--ignore-cache") {
            if ignore_cache {
                return Err("duplicate --ignore-cache".to_string());
            }
            ignore_cache = true;
        } else {
            numbers.push(arg.clone());
        }
    }
    Ok(CheckOptions {
        selected: select_expectations(config, &numbers)?,
        fail_fast,
        ignore_cache,
    })
}

pub(crate) fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    if config.agent.instructions.trim().is_empty() {
        return Err("check.yml agent.instructions must not be empty".to_string());
    }
    validate_optional_model(config.agent.model.primary.as_deref(), "agent.model.primary")?;
    for (index, model) in config.agent.model.fallbacks.iter().enumerate() {
        validate_optional_model(
            Some(model.as_str()),
            &format!("agent.model.fallbacks[{}]", index),
        )?;
    }
    validate_thinking(&config.agent.thinking)?;
    for path in &config.agent.ignore {
        validate_relative_config_path(path, "agent ignore pattern")?;
    }
    for plugin in &config.agent.plugins {
        validate_plugin_config_key(plugin)?;
    }
    if config.expectations.is_empty() {
        return Err("check.yml expectations must not be empty".to_string());
    }
    for (index, expectation) in config.expectations.iter().enumerate() {
        let number = index + 1;
        if expectation.q.trim().is_empty() {
            return Err(format!("expectation {} has an empty q", number));
        }
        if expectation.a.contains('\n') || expectation.a.contains('\r') {
            return Err(format!(
                "expectation {} expected answer must be single-line",
                number
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_plugin_config_key(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("agent has an empty plugin entry".to_string());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("agent plugin entries must be single-line strings".to_string());
    }
    if !value.contains('@') {
        return Err(format!(
            "agent plugin entry must use Codex plugin key <plugin>@<marketplace>: {}",
            value
        ));
    }
    Ok(())
}

pub(crate) fn validate_optional_model(value: Option<&str>, label: &str) -> Result<(), String> {
    let Some(model) = value else {
        return Ok(());
    };
    if model.trim().is_empty() {
        return Err(format!("check.yml {} must not be empty", label));
    }
    if model.contains('\n') || model.contains('\r') {
        return Err(format!("check.yml {} must be a single-line string", label));
    }
    Ok(())
}

pub(crate) fn validate_thinking(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("check.yml agent.thinking must not be empty".to_string());
    }
    if value.contains('\n') || value.contains('\r') {
        return Err("check.yml agent.thinking must be a single-line string".to_string());
    }
    match value {
        "off" | "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "adaptive" | "max" => {
            Ok(())
        }
        _ => Err(format!("unsupported check.yml agent.thinking: {}", value)),
    }
}

pub(crate) fn codex_reasoning_effort(thinking: &str) -> Option<&str> {
    match thinking {
        "adaptive" => None,
        "off" | "none" => Some("none"),
        "max" => Some("xhigh"),
        value => Some(value),
    }
}

pub(crate) fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
}

pub(crate) fn validate_relative_config_path(value: &str, label: &str) -> Result<(), String> {
    normalize_repo_path(value)
        .map(|_| ())
        .map_err(|err| format!("{}: {}", label, err))
}

pub(crate) fn select_expectations(
    config: &CheckConfig,
    args: &[OsString],
) -> Result<Vec<SelectedExpectation>, String> {
    let mut selected_numbers = Vec::new();
    if args.is_empty() {
        selected_numbers.extend(1..=config.expectations.len());
    } else {
        let mut seen = BTreeSet::new();
        for arg in args {
            let text = arg
                .to_str()
                .ok_or("expectation number must be valid UTF-8".to_string())?;
            let number = text
                .parse::<usize>()
                .map_err(|_| format!("invalid expectation number: {}", text))?;
            if number == 0 {
                return Err("expectation numbers are 1-based".to_string());
            }
            if number > config.expectations.len() {
                return Err(format!("expectation number out of range: {}", number));
            }
            if !seen.insert(number) {
                return Err(format!("duplicate expectation number: {}", number));
            }
            selected_numbers.push(number);
        }
    }

    Ok(selected_numbers
        .into_iter()
        .map(|number| {
            let expectation = &config.expectations[number - 1];
            SelectedExpectation {
                number,
                id: expectation_id(&expectation.q, &expectation.a),
                q: expectation.q.clone(),
                a: expectation.a.clone(),
            }
        })
        .collect())
}

pub(crate) fn fail_on_mixed_canon_changes(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--cached")
        .arg("--name-only")
        .arg("--diff-filter=ACDMRTUXB")
        .output()
        .map_err(|err| format!("failed to run git diff: {}", err))?;
    if !output.status.success() {
        return Err("failed to inspect staged git changes".to_string());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "git diff output must be valid UTF-8".to_string())?;
    let paths = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    fail_on_mixed_canon_paths(&paths)
}

pub(crate) fn fail_on_mixed_canon_paths(paths: &[String]) -> Result<(), String> {
    let has_canon = paths.iter().any(|path| is_canon_project_path(path));
    let has_other = paths.iter().any(|path| !is_canon_project_path(path));
    if has_canon && has_other {
        return Err(
            "canon check failed: .canon/** changes must not be mixed with non-.canon changes"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn is_canon_project_path(path: &str) -> bool {
    path == ".canon" || path.starts_with(".canon/")
}

pub(crate) fn run_check_with_runner<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    options: &CheckOptions,
    runner: &mut R,
    mut diagnostic_log: Option<&mut DiagnosticLogWriter>,
    mut result_output: Option<&mut dyn Write>,
) -> Result<Vec<CheckRecord>, String> {
    let mut records = Vec::new();
    let mut sessions = BTreeMap::new();
    let mut scope_hash_cache = ScopeHashCache::new();
    let mut history_cache = HistoryCache::new();
    let mut parse_cache = EvaluatorResponseParseCache::new();
    for expectation in &options.selected {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        if !options.ignore_cache {
            if let Some(record) = reusable_history_record_for_source(
                root,
                &config.agent,
                expectation,
                ScopeHashSource::Index,
                &mut history_cache,
                &mut scope_hash_cache,
            )? {
                // Cache hits are result reuse, not evaluator interrogations, so
                // they print stdout but do not create diagnostic interrogation logs.
                let should_stop = options.fail_fast && !record.passed();
                write_and_flush_result_output(&mut result_output, &record)?;
                records.push(record);
                if should_stop {
                    return Ok(records);
                }
                continue;
            }
        }

        let scope =
            latest_history_scope_with_cache(root, &config.agent, expectation, &mut history_cache)?
                .unwrap_or_else(full_scope);
        let mut enforced_scope = scope.clone();
        let mut interrogation = interrogate_expectation(
            root,
            snapshot_root,
            config,
            expectation,
            runner,
            &mut sessions,
            &mut diagnostic_log,
            &mut scope_hash_cache,
            &mut parse_cache,
            &enforced_scope,
        )?;
        if should_retry_full_scope(&interrogation.record, &enforced_scope) {
            // Widening after a restricted idk is not narrowing verification:
            // it is a separate full-scope interrogation whose record replaces
            // the restricted non-answer.
            enforced_scope = full_scope();
            interrogation = interrogate_expectation(
                root,
                snapshot_root,
                config,
                expectation,
                runner,
                &mut sessions,
                &mut diagnostic_log,
                &mut scope_hash_cache,
                &mut parse_cache,
                &enforced_scope,
            )?;
        }

        let record_scope = interrogation.record.scope.clone();
        let mut write_history = true;
        if is_strict_scope_subset(&record_scope, &enforced_scope) {
            // A narrower scope from one evaluator response becomes reusable
            // only if an independent interrogation with that same canonical
            // scope preserves the answer.
            let narrowed = interrogate_expectation(
                root,
                snapshot_root,
                config,
                expectation,
                runner,
                &mut sessions,
                &mut diagnostic_log,
                &mut scope_hash_cache,
                &mut parse_cache,
                &record_scope,
            )?;
            if narrowed.record.observed == interrogation.record.observed {
                interrogation = narrowed;
            } else {
                write_history = false;
            }
        }

        if write_history && is_verified_record(&interrogation.record) {
            append_history_record_with_cache(
                root,
                expectation,
                &interrogation.record,
                &mut history_cache,
            )?;
        }
        let should_stop = options.fail_fast && !interrogation.record.passed();
        write_and_flush_result_output(&mut result_output, &interrogation.record)?;
        records.push(interrogation.record);
        if should_stop {
            return Ok(records);
        }
    }
    Ok(records)
}

pub(crate) fn should_retry_full_scope(record: &CheckRecord, scope: &[String]) -> bool {
    scope != full_scope() && record.observed == "idk"
}

pub(crate) fn interrogate_expectation<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    expectation: &SelectedExpectation,
    runner: &mut R,
    sessions: &mut BTreeMap<String, String>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    scope_hash_cache: &mut ScopeHashCache,
    parse_cache: &mut EvaluatorResponseParseCache,
    enforced_scope: &[String],
) -> Result<InterrogationResult, String> {
    let mut failures = Vec::new();
    for model in evaluator_models(&config.agent) {
        if check_interrupted() {
            return Err("interrupted".to_string());
        }
        match interrogate_expectation_with_model(
            root,
            snapshot_root,
            config,
            expectation,
            runner,
            sessions,
            diagnostic_log,
            scope_hash_cache,
            parse_cache,
            enforced_scope,
            model.as_deref(),
        ) {
            Ok(result) => return Ok(result),
            Err(err) if is_model_technical_failure(&err) => {
                failures.push(format!("{}: {}", model_label(model.as_deref()), err));
            }
            Err(err) => return Err(err),
        }
    }
    Err(format!(
        "all evaluator models failed: {}",
        failures.join("; ")
    ))
}

pub(crate) fn interrogate_expectation_with_model<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    expectation: &SelectedExpectation,
    runner: &mut R,
    sessions: &mut BTreeMap<String, String>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    scope_hash_cache: &mut ScopeHashCache,
    parse_cache: &mut EvaluatorResponseParseCache,
    enforced_scope: &[String],
    model: Option<&str>,
) -> Result<InterrogationResult, String> {
    let enforced_scope = sanitize_scope(enforced_scope, &config.agent)?;
    // Threads are reused only for the same canonical enforced scope. When
    // checking a narrower scope returned by an evaluator response, that
    // canonical record scope is passed back here unchanged as enforced_scope.
    let session_key = format!("{}\n{}", model_label(model), enforced_scope.join("\n"));
    let existing_session = sessions.get(&session_key).cloned();
    let session_id = match existing_session {
        Some(existing) => existing,
        None => runner.start_session(
            snapshot_root,
            &developer_instructions(&config.agent),
            &config.agent,
            model,
            &enforced_scope,
        )?,
    };
    let prompt = question_prompt(&expectation.q, &enforced_scope)?;
    let response = match ask_with_repairs(runner, &session_id, &prompt, &config.agent, parse_cache)
    {
        Ok(response) => response,
        Err(err) => {
            if is_model_technical_failure(&err) {
                sessions.remove(&session_key);
            }
            return Err(err);
        }
    };
    sessions
        .entry(session_key)
        .or_insert_with(|| session_id.clone());
    let record_scope = response.scope.clone();
    let scope_hash = scope_hash_cache.staged_scope_hash(root, &config.agent, &record_scope)?;
    let record = record_from_response(expectation, response, record_scope, scope_hash)?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_record(&record)?;
    }
    Ok(InterrogationResult { record })
}

pub(crate) fn evaluator_models(agent: &AgentConfig) -> Vec<Option<String>> {
    let mut models = vec![agent.model.primary.clone()];
    models.extend(agent.model.fallbacks.iter().cloned().map(Some));
    models
}

pub(crate) fn model_label(model: Option<&str>) -> &str {
    model.unwrap_or("<default>")
}

pub(crate) fn is_model_technical_failure(err: &str) -> bool {
    err.contains("usageLimitExceeded")
        || err.contains("usage limit")
        || err.contains("rate limit")
        || err.contains("model unavailable")
        || err.contains("model is unavailable")
        || err.contains("timed out")
}

pub(crate) fn record_from_response(
    expectation: &SelectedExpectation,
    response: ParsedAnswer,
    enforced_scope: Vec<String>,
    scope_hash: String,
) -> Result<CheckRecord, String> {
    if response.answer == "malformed" {
        eprintln!(
            "canon check: expectation {}: {}",
            expectation.number, MALFORMED_REVIEW_WARNING
        );
    }
    if response.evidence.trim().is_empty() {
        eprintln!(
            "canon check: expectation {}: evidence is empty after retry",
            expectation.number
        );
    }
    let result = if response.answer == expectation.a && response.answer != "malformed" {
        "pass"
    } else {
        "fail"
    };
    Ok(CheckRecord {
        timestamp: format_log_record_timestamp(unix_timestamp()?),
        number: expectation.number,
        result: result.to_string(),
        prompt: expectation.q.clone(),
        expected: expectation.a.clone(),
        observed: response.answer,
        evidence: response.evidence,
        scope: enforced_scope,
        scope_hash,
    })
}

pub(crate) fn is_verified_record(record: &CheckRecord) -> bool {
    record.observed != UNPARSEABLE_OBSERVED
}

pub(crate) fn ask_with_repairs<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    prompt: &str,
    agent: &AgentConfig,
    parser_cache: &mut EvaluatorResponseParseCache,
) -> Result<ParsedAnswer, String> {
    let first = runner.ask(session_id, prompt)?;
    let mut parsed = match parser_cache.parse(&first, agent) {
        Ok(answer) => answer,
        Err(_err) => {
            let first_excerpt = response_excerpt(&first);
            let repaired = runner.ask(session_id, prompt)?;
            match parser_cache.parse(&repaired, agent) {
                Ok(answer) => answer,
                Err(err) => ParsedAnswer {
                    answer: UNPARSEABLE_OBSERVED.to_string(),
                    evidence: format!(
                        "evaluator response could not be parsed after retry: {}\nfirst response: {}\nrepair response: {}",
                        err,
                        first_excerpt,
                        response_excerpt(&repaired)
                    ),
                    scope: full_scope(),
                },
            }
        }
    };

    if parsed.answer == "malformed" {
        let repaired = runner.ask(session_id, prompt)?;
        if let Ok(answer) = parser_cache.parse(&repaired, agent) {
            parsed = answer;
        }
    }

    if parsed.answer == "idk" && should_repair_absence_idk(prompt) {
        let repaired = runner.ask(session_id, prompt)?;
        if let Ok(answer) = parser_cache.parse(&repaired, agent) {
            parsed = answer;
        }
    }

    if parsed.evidence.trim().is_empty() {
        let repaired = runner.ask(session_id, prompt)?;
        if let Ok(answer) = parser_cache.parse(&repaired, agent) {
            parsed = answer;
        }
    }

    Ok(parsed)
}

#[derive(Default)]
pub(crate) struct EvaluatorResponseParseCache {
    values: BTreeMap<(String, Vec<String>), Result<ParsedAnswer, String>>,
}

impl EvaluatorResponseParseCache {
    pub(crate) fn new() -> EvaluatorResponseParseCache {
        EvaluatorResponseParseCache::default()
    }

    pub(crate) fn parse(
        &mut self,
        text: &str,
        agent: &AgentConfig,
    ) -> Result<ParsedAnswer, String> {
        let key = (text.to_string(), effective_ignore_patterns(agent));
        if let Some(parsed) = self.values.get(&key) {
            return parsed.clone();
        }
        let parsed = parse_evaluator_response(text, agent);
        self.values.insert(key, parsed.clone());
        parsed
    }
}

pub(crate) fn response_excerpt(text: &str) -> String {
    const LIMIT: usize = 600;
    let text = text.trim();
    if text.is_empty() {
        return "<empty>".to_string();
    }
    let mut excerpt = text.chars().take(LIMIT).collect::<String>();
    if text.chars().count() > LIMIT {
        excerpt.push_str("...");
    }
    excerpt
}

pub(crate) fn question_prompt(question: &str, scope: &[String]) -> Result<String, String> {
    serde_json::to_string(&json!({
        "scope": scope,
        "question": question,
    }))
    .map_err(|err| format!("failed to serialize evaluator question prompt: {}", err))
}

pub(crate) fn developer_instructions(agent: &AgentConfig) -> String {
    format!(
        "{}\n\n{}",
        agent.instructions.trim(),
        response_format_block()
    )
}

pub(crate) fn should_repair_absence_idk(prompt: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(prompt.trim()) else {
        return false;
    };
    let Some(question) = value.get("question").and_then(Value::as_str) else {
        return false;
    };
    question.starts_with("Are there any ")
        || question.starts_with("Is there any ")
        || question.starts_with("Can any ")
}

pub(crate) fn response_format_block() -> &'static str {
    concat!(
        "Response format:\n",
        "Return exactly one valid JSON object and no markdown, code fences, or surrounding prose.\n",
        "Schema: {\"answer\":\"<single-line answer>\",\"evidence\":\"<free-form evidence citing supporting files or code>\",\"scope\":[\"<normalized repository-relative path>\"]}\n",
        "`scope` is the smallest allowed project context sufficient to answer this question with the same `answer`; it is not the list of evidence citations. ",
        "Use [\".\"] when the answer depends on project-wide absence, consistency, duplication, garbage, overall quality, or denied/inaccessible paths. ",
        "If the enforced task `scope` is narrower than [\".\"] and the question requires repository-wide or cross-module evidence, answer `idk` instead of drawing a positive or negative conclusion from incomplete context. ",
        "Never include denied or inaccessible paths in `scope`. Denied paths are intentionally outside the allowed evidence boundary; do not answer `idk` solely because a denied path is unreadable. ",
        "The current project state is the staged Git snapshot exposed at the working directory; do not treat files that exist only in `HEAD`, cache history, or diagnostic logs as current project files. ",
        "The user-provided `agent.instructions` above are active project policy loaded from `.canon/check.yml`, not hardcoded implementation text and not necessarily the embedded default template shown in README; do not cite those instructions as `src/check.rs` contents. ",
        "For questions about copying the staged Git snapshot, `copy` means creating a duplicate repository/tree/snapshot directory; an in-place index view that uses `git stash --keep-index` only to preserve unstaged worktree changes is not a copy. ",
        "A reusable cache hit is not an evaluator interrogation; questions about every evaluator interrogation concern only turns where `canon check` actually asks the evaluator model. ",
        "For `.canon/check.yml` schema/configuration questions, do not try to answer by opening `.canon/check.yml`; that path is denied by design. Use the fact that `canon check` has already loaded and validated the active config before starting the evaluator, plus the visible README, parser, validation, and template code; do not answer `idk` solely because `.canon/check.yml` itself is denied. ",
        "For absence and quality questions, answer `yes` only when there is a concrete removable file, code path, hack, or idiom violation with evidence; answer `no` when repository-wide inspection finds no concrete candidate, because absolute proof of absence is not required. ",
        "Treat behavior required by the active check contract, such as staged-snapshot restoration, process-tree cleanup, and configured log rolling, as not avoidable by itself.\n",
    )
}

pub(crate) fn parse_evaluator_response(
    text: &str,
    agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let response = match parse_evaluator_response_json(text) {
        Ok(response) => response,
        Err(err) => {
            if let Some(answer) = parse_plain_evaluator_answer(text) {
                return Ok(ParsedAnswer {
                    answer,
                    evidence: String::new(),
                    scope: full_scope(),
                });
            }
            return Err(err);
        }
    };
    if response.answer.contains('\n') || response.answer.contains('\r') {
        return Err("answer must be a single-line string".to_string());
    }
    Ok(ParsedAnswer {
        answer: response.answer,
        evidence: response.evidence,
        scope: parse_scope_strings(&response.scope, agent)?,
    })
}

pub(crate) fn parse_plain_evaluator_answer(text: &str) -> Option<String> {
    let answer = text.trim();
    if answer.contains('\n') || answer.contains('\r') {
        return None;
    }
    if matches!(answer, "yes" | "no" | "idk" | "malformed")
        || (answer.len() == 1 && answer.as_bytes()[0].is_ascii_lowercase())
    {
        return Some(answer.to_string());
    }
    None
}

pub(crate) fn parse_evaluator_response_json(text: &str) -> Result<EvaluatorResponseJson, String> {
    let trimmed = text.trim();
    match serde_json::from_str::<EvaluatorResponseJson>(trimmed) {
        Ok(response) => Ok(response),
        Err(strict_err) => {
            let Some(object) = first_json_object(trimmed) else {
                return Err(format!(
                    "failed to parse evaluator JSON response: {}",
                    strict_err
                ));
            };
            serde_json::from_str::<EvaluatorResponseJson>(object).map_err(|embedded_err| {
                format!(
                    "failed to parse evaluator JSON response: {}; embedded JSON parse failed: {}",
                    strict_err, embedded_err
                )
            })
        }
    }
}

pub(crate) fn first_json_object(text: &str) -> Option<&str> {
    let start = text.char_indices().find(|(_, ch)| *ch == '{')?.0;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (relative_index, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    let end = start + relative_index + ch.len_utf8();
                    return Some(&text[start..end]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn parse_scope_json(text: &str, agent: &AgentConfig) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| format!("failed to parse SCOPE JSON: {}", err))?;
    let array = value
        .as_array()
        .ok_or("SCOPE must be a JSON array".to_string())?;
    let mut scope = Vec::new();
    for item in array {
        let raw = item
            .as_str()
            .ok_or("SCOPE entries must be strings".to_string())?;
        let normalized = normalize_repo_path(raw)?;
        if normalized != raw.trim() {
            return Err(format!("SCOPE entry must be normalized: {}", raw));
        }
        scope.push(normalized);
    }
    sanitize_scope(&scope, agent)
}

pub(crate) fn parse_scope_strings(
    scope: &[String],
    agent: &AgentConfig,
) -> Result<Vec<String>, String> {
    let mut parsed = Vec::new();
    for raw in scope {
        let normalized = normalize_repo_path(raw)?;
        if normalized != raw.trim() {
            return Err(format!("scope entry must be normalized: {}", raw));
        }
        if normalized != "." && is_denied_path(agent, &normalized) {
            continue;
        }
        parsed.push(normalized);
    }
    sanitize_scope(&parsed, agent)
}

pub(crate) fn write_and_flush_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
        // This is the real-time stdout boundary: one complete JSONL result is
        // written and flushed before the next expectation is evaluated.
        let line = render_check_log_record(record);
        writer
            .write_all(line.as_bytes())
            .map_err(|err| format!("failed to write check result to stdout: {}", err))?;
        writer
            .flush()
            .map_err(|err| format!("failed to flush check result to stdout: {}", err))?;
    }
    Ok(())
}

impl CheckRecord {
    pub(crate) fn passed(&self) -> bool {
        self.result == "pass"
    }
}
