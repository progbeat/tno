fn run_check_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    let command = parse_check_command_args(args)?;
    let config = load_check_config(root, &command.config_path)?;
    let options = parse_check_options(&config, &command.option_args)?;
    fail_on_mixed_canon_changes(root)?;
    let tree = git_write_tree(root)?;
    let snapshot = StagedSnapshot::create(root, &tree)?;
    let mut runner = LazyAppServerRunner::new(check_config_loads_plugins(&config));
    let mut diagnostic_log = DiagnosticLogWriter::create(root)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut result_output: &mut dyn Write = &mut stdout;
    let records = run_check_with_runner(
        root,
        snapshot.path(),
        &config,
        &options,
        &mut runner,
        Some(&mut diagnostic_log),
        Some(&mut result_output),
    )?;
    diagnostic_log.finish()?;
    if records.iter().all(CheckRecord::passed) {
        Ok(())
    } else {
        Err("canon check failed".to_string())
    }
}

fn run_gate_command(root: &Path, args: &[OsString]) -> Result<(), String> {
    if args.iter().any(|arg| arg.to_str() == Some("--fail-fast")) {
        return Err("canon gate does not accept --fail-fast".to_string());
    }
    if args
        .iter()
        .any(|arg| arg.to_str() == Some("--ignore-cache"))
    {
        return Err("canon gate does not accept --ignore-cache".to_string());
    }
    let config = load_check_config(root, Path::new(CHECK_PATH))?;
    let selected = select_expectations(&config, args)?;
    fail_on_mixed_canon_changes(root)?;

    let mut missing = Vec::new();
    let mut failing = Vec::new();
    for expectation in &selected {
        match reusable_history_record(root, &config.agent, expectation)? {
            Some(record) if record.passed() => {}
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

fn parse_check_command_args(args: &[OsString]) -> Result<CheckCommandArgs, String> {
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
            config_path = Some(PathBuf::from(value));
        } else if let Some(value) = arg.strip_prefix("--config=") {
            if config_path.is_some() {
                return Err("duplicate --config".to_string());
            }
            if value.is_empty() {
                return Err("--config requires a path".to_string());
            }
            config_path = Some(PathBuf::from(value));
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

fn load_check_config(root: &Path, config_path: &Path) -> Result<CheckConfig, String> {
    let content = if config_path == Path::new(CHECK_PATH) {
        staged_file_content(root, CHECK_PATH).or_else(|_| {
            let path = root.join(CHECK_PATH);
            fs::read_to_string(&path)
                .map_err(|err| format!("failed to read {}: {}", path.display(), err))
        })
    } else {
        let path = root.join(config_path);
        fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {}", path.display(), err))
    }?;
    let config: CheckConfig = serde_yaml::from_str(&content)
        .map_err(|err| format!("failed to parse {}: {}", config_path.display(), err))?;
    validate_check_config(&config)?;
    Ok(config)
}

fn parse_check_options(config: &CheckConfig, args: &[OsString]) -> Result<CheckOptions, String> {
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

fn validate_check_config(config: &CheckConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("check.yml version must be 1".to_string());
    }
    if config.agent.instructions.trim().is_empty() {
        return Err("check.yml agent.instructions must not be empty".to_string());
    }
    if let Some(model) = &config.agent.model {
        if model.trim().is_empty() {
            return Err("check.yml agent.model must not be empty".to_string());
        }
        if model.contains('\n') || model.contains('\r') {
            return Err("check.yml agent.model must be a single-line string".to_string());
        }
    }
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

fn validate_plugin_config_key(value: &str) -> Result<(), String> {
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

fn check_config_loads_plugins(config: &CheckConfig) -> bool {
    !config.agent.plugins.is_empty()
}

fn validate_relative_config_path(value: &str, label: &str) -> Result<(), String> {
    normalize_repo_path(value)
        .map(|_| ())
        .map_err(|err| format!("{}: {}", label, err))
}

fn select_expectations(
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

fn fail_on_mixed_canon_changes(root: &Path) -> Result<(), String> {
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

fn fail_on_mixed_canon_paths(paths: &[String]) -> Result<(), String> {
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

fn is_canon_project_path(path: &str) -> bool {
    path == ".canon" || path.starts_with(".canon/")
}

fn run_check_with_runner<R: EvaluatorRunner>(
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
    for expectation in &options.selected {
        if !options.ignore_cache {
            if let Some(record) = reusable_history_record(root, &config.agent, expectation)? {
                let should_stop = options.fail_fast && !record.passed();
                write_result_output(&mut result_output, &record)?;
                records.push(record);
                if should_stop {
                    return Ok(records);
                }
                continue;
            }
        }

        let mut enforced_scope =
            latest_history_scope(root, &config.agent, expectation)?.unwrap_or_else(full_scope);
        let mut interrogation = interrogate_expectation(
            root,
            snapshot_root,
            config,
            expectation,
            runner,
            &mut sessions,
            &mut diagnostic_log,
            &enforced_scope,
        )?;
        if interrogation.record.observed == "idk" && enforced_scope != full_scope() {
            enforced_scope = full_scope();
            // Widening after a restricted `idk` is not a narrowing verification:
            // the full-scope answer replaces the restricted `idk`.
            interrogation = interrogate_expectation(
                root,
                snapshot_root,
                config,
                expectation,
                runner,
                &mut sessions,
                &mut diagnostic_log,
                &enforced_scope,
            )?;
        }

        let proposed_scope = sanitize_scope(&interrogation.proposed_scope, &config.agent)
            .unwrap_or_else(|_| interrogation.record.scope.clone());
        if is_strict_scope_subset(&proposed_scope, &enforced_scope) {
            // A proposed narrower scope is never written into the current
            // record directly. It becomes reusable only if an independent
            // interrogation with that enforced scope preserves the answer.
            let narrowed = interrogate_expectation(
                root,
                snapshot_root,
                config,
                expectation,
                runner,
                &mut sessions,
                &mut diagnostic_log,
                &proposed_scope,
            )?;
            if narrowed.record.observed == interrogation.record.observed {
                interrogation = narrowed;
            }
        }

        append_history_record(root, expectation, &interrogation.record)?;
        let should_stop = options.fail_fast && !interrogation.record.passed();
        write_result_output(&mut result_output, &interrogation.record)?;
        records.push(interrogation.record);
        if should_stop {
            return Ok(records);
        }
    }
    Ok(records)
}

fn interrogate_expectation<R: EvaluatorRunner>(
    root: &Path,
    snapshot_root: &Path,
    config: &CheckConfig,
    expectation: &SelectedExpectation,
    runner: &mut R,
    sessions: &mut BTreeMap<String, String>,
    diagnostic_log: &mut Option<&mut DiagnosticLogWriter>,
    enforced_scope: &[String],
) -> Result<InterrogationResult, String> {
    let scope = sanitize_scope(enforced_scope, &config.agent)?;
    let scope_hash = staged_scope_hash(root, &config.agent, &scope)?;
    let session_key = scope.join("\n");
    let session_id = if let Some(existing) = sessions.get(&session_key) {
        existing.clone()
    } else {
        let session_id = runner.start_session(
            snapshot_root,
            &config.agent.instructions,
            &config.agent,
            &scope,
        )?;
        sessions.insert(session_key, session_id.clone());
        session_id
    };
    let prompt = question_prompt(config, &expectation.q, &scope);
    let response = ask_with_repairs(runner, &session_id, &prompt, &config.agent)?;
    let proposed_scope = response.scope.clone();
    let record = record_from_response(expectation, response, scope, scope_hash)?;
    if let Some(writer) = diagnostic_log.as_deref_mut() {
        writer.write_record(&record)?;
    }
    Ok(InterrogationResult {
        record,
        proposed_scope,
    })
}

fn record_from_response(
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

fn ask_with_repairs<R: EvaluatorRunner>(
    runner: &mut R,
    session_id: &str,
    prompt: &str,
    agent: &AgentConfig,
) -> Result<ParsedAnswer, String> {
    let first = runner.ask(session_id, prompt)?;
    let mut parsed = match parse_evaluator_response(&first, agent) {
        Ok(answer) => answer,
        Err(err) => {
            let repaired = runner.ask(session_id, &malformed_repair_prompt(&err))?;
            match parse_evaluator_response(&repaired, agent) {
                Ok(answer) => answer,
                Err(err) => ParsedAnswer {
                    answer: "malformed".to_string(),
                    evidence: format!("evaluator response could not be parsed after retry: {}", err),
                    scope: full_scope(),
                },
            }
        }
    };

    if parsed.answer == "malformed" {
        let repaired = runner.ask(session_id, malformed_answer_repair_prompt())?;
        if let Ok(answer) = parse_evaluator_response(&repaired, agent) {
            parsed = answer;
        }
    }

    if parsed.evidence.trim().is_empty() {
        let repaired = runner.ask(session_id, evidence_repair_prompt())?;
        if let Ok(answer) = parse_evaluator_response(&repaired, agent) {
            parsed = answer;
        }
    }

    Ok(parsed)
}

fn question_prompt(config: &CheckConfig, question: &str, scope: &[String]) -> String {
    format!(
        "{}\n\nRuntime check.yml metadata summary:\n{}\nThis summary is provided by canon itself, is not file-read access to `.canon/check.yml`, and does not include expected answers.\n\nAllowed scope:\n{}\n\nExpectation:\n{}\n\nReply using this exact format:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence citing supporting files or code>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths representing the smallest allowed project context sufficient to answer this expectation with the same ANSWER; this is not the list of evidence citations; use [\".\"] when the answer depends on project-wide absence, consistency, duplication, garbage, or overall quality>\n",
        config.agent.instructions.trim(),
        check_config_summary(config),
        serde_json::to_string(scope).expect("scope is serializable"),
        question
    )
}

fn check_config_summary(config: &CheckConfig) -> String {
    format!(
        "- version: {}\n- top-level `agent` section: present\n- top-level `agents` section: absent\n- single evaluator agent fields: instructions, model, ignore, plugins\n- configured model: {}\n- configured ignore patterns: {}\n- configured plugins: {}\n- expectation count: {}",
        config.version,
        config.agent.model.as_deref().unwrap_or("<default>"),
        serde_json::to_string(&config.agent.ignore).expect("ignore patterns are serializable"),
        serde_json::to_string(&config.agent.plugins).expect("plugins are serializable"),
        config.expectations.len()
    )
}

fn malformed_repair_prompt(error: &str) -> String {
    format!(
        "Your previous response could not be parsed: {}. Reply again using exactly:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence citing supporting files or code>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths representing the smallest allowed project context sufficient to answer with the same ANSWER; this is not the list of evidence citations; do not include denied paths such as .canon/**, .git/**, or configured ignore paths; use [\".\"] when no narrower allowed scope is sufficient>\n",
        error
    )
}

fn malformed_answer_repair_prompt() -> &'static str {
    "Your previous answer was `malformed`. Retry once. If the question is truly malformed, answer `malformed` again. Reply using exactly:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence citing supporting files or code>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths representing the smallest allowed project context sufficient to answer with the same ANSWER; this is not the list of evidence citations>\n"
}

fn evidence_repair_prompt() -> &'static str {
    "Your previous response had an answer but no evidence. Reply again with the same format and include evidence if the available files support it:\nANSWER: <single-line answer>\nEVIDENCE:\n<free-form evidence citing supporting files or code>\nSCOPE: <JSON array of up to 4 normalized repository-relative paths representing the smallest allowed project context sufficient to answer with the same ANSWER; this is not the list of evidence citations>\n"
}

fn parse_evaluator_response(text: &str, agent: &AgentConfig) -> Result<ParsedAnswer, String> {
    let lines = text.lines().collect::<Vec<_>>();
    let answer_line_index = lines
        .iter()
        .position(|line| line.trim_start().starts_with("ANSWER: "))
        .ok_or("missing ANSWER line".to_string())?;
    let answer = lines[answer_line_index]
        .trim_start()
        .trim_end()
        .strip_prefix("ANSWER: ")
        .ok_or("missing ANSWER line".to_string())?
        .to_string();
    let evidence_line_index = lines
        .iter()
        .enumerate()
        .skip(answer_line_index + 1)
        .find_map(|(index, line)| {
            line.trim_start()
                .starts_with("EVIDENCE:")
                .then_some(index)
        })
        .ok_or("missing EVIDENCE block".to_string())?;
    let scope_line_index = lines
        .iter()
        .enumerate()
        .skip(evidence_line_index + 1)
        .rev()
        .find_map(|(index, line)| line.trim_start().strip_prefix("SCOPE: ").map(|_| index))
        .ok_or("missing SCOPE line".to_string())?;
    let scope_text = lines[scope_line_index]
        .trim_start()
        .trim_end()
        .strip_prefix("SCOPE: ")
        .ok_or("missing SCOPE line".to_string())?;
    let evidence_header = lines[evidence_line_index].trim_start();
    let evidence_suffix = evidence_header
        .strip_prefix("EVIDENCE:")
        .unwrap_or("")
        .trim_start();
    let mut evidence_lines = Vec::new();
    if !evidence_suffix.is_empty() {
        evidence_lines.push(evidence_suffix);
    }
    evidence_lines.extend(lines[evidence_line_index + 1..scope_line_index].iter().copied());
    Ok(ParsedAnswer {
        answer,
        evidence: evidence_lines.join("\n"),
        scope: parse_scope_json(scope_text, agent)?,
    })
}

fn parse_scope_json(text: &str, agent: &AgentConfig) -> Result<Vec<String>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| format!("failed to parse SCOPE JSON: {}", err))?;
    let array = value
        .as_array()
        .ok_or("SCOPE must be a JSON array".to_string())?;
    if array.len() > 4 {
        return Err("SCOPE must contain at most 4 paths".to_string());
    }
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

fn write_result_output(
    result_output: &mut Option<&mut dyn Write>,
    record: &CheckRecord,
) -> Result<(), String> {
    if let Some(writer) = result_output.as_mut() {
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
    fn passed(&self) -> bool {
        self.result == "pass"
    }
}
