use super::*;

fn input(response: Option<Value>) -> Input {
    let file = RequestFile {
        method: "POST".into(),
        url: "{{baseUrl}}/users?page=2".into(),
        headers: vec![KeyValue {
            name: "Accept".into(),
            value: "application/json".into(),
            enabled: true,
        }],
        body: Body::Json {
            content: r#"{"name":"Ann"}"#.into(),
        },
        ..Default::default()
    };
    let mut vars = Vars::default();
    vars.environment.name = Some("Local".into());
    vars.environment
        .values
        .insert("baseUrl".into(), "http://localhost:3000".into());
    vars.collection.insert("retries".into(), "3".into());
    vars.globals.insert("who".into(), "globals".into());
    Input {
        info: Info {
            request_name: "Create user".into(),
            iteration: 0,
            iteration_count: 1,
            ..Default::default()
        },
        request: request_value(&file, "Create user", "Users/Create user.yaml"),
        response,
        vars,
        vault: BTreeMap::from([("apiKey".into(), "s3cret".into())]),
    }
}

fn response() -> Value {
    json!({
        "code": 201,
        "status": "Created",
        "headers": [
            { "key": "Content-Type", "value": "application/json" },
            { "key": "Set-Cookie", "value": "session=abc; Path=/; HttpOnly" },
        ],
        "body": r#"{"id":7,"name":"Ann","tags":["a","b"]}"#,
        "responseTime": 42,
        "responseSize": 38,
    })
}

fn test(source: &str) -> Output {
    run(
        Event::Test,
        &[Script {
            label: "request".into(),
            source: source.into(),
        }],
        &input(Some(response())),
        None,
        DEFAULT_TIMEOUT,
    )
}

fn summary(output: &Output) -> Vec<(String, bool)> {
    output
        .tests
        .iter()
        .map(|t| (t.name.clone(), t.passed))
        .collect()
}

#[test]
fn postman_style_tests() {
    let output = test(
        r#"
        pm.test("Status is 201", () => pm.response.to.have.status(201));
        pm.test("Is created", function () { pm.expect(pm.response.code).to.equal(201); });
        pm.test("Has a JSON id", () => {
            const json = pm.response.json();
            pm.expect(json.id).to.be.a("number");
            pm.expect(json).to.have.property("name", "Ann");
            pm.expect(json.tags).to.include("b");
        });
        pm.test("Header", () => pm.response.to.have.header("Content-Type"));
        pm.test("Fast", () => pm.expect(pm.response.responseTime).to.be.below(200));
        pm.test("Body path", () => pm.response.to.have.jsonBody("tags[1]", "b"));
        pm.test("Is success", () => pm.response.to.be.success);
        pm.test("Wrong status", () => pm.response.to.have.status(200));
        pm.test("Not an error", () => pm.response.to.not.be.error);
        pm.test("Schema", () => pm.response.to.have.jsonSchema({ type: "object", required: ["id"], properties: { id: { type: "integer" } } }));
        pm.test("Cookie", () => pm.expect(pm.cookies.get("session")).to.eql("abc"));
        pm.test("Later");
        "#,
    );
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    let s = summary(&output);
    assert_eq!(s.len(), 12);
    assert!(
        s.iter()
            .filter(|(n, _)| n != "Wrong status")
            .all(|(_, p)| *p),
        "{:?}",
        output.tests
    );
    let wrong = output
        .tests
        .iter()
        .find(|t| t.name == "Wrong status")
        .unwrap();
    assert!(!wrong.passed);
    assert_eq!(
        wrong.error.as_deref(),
        Some("AssertionError: expected response to have status code 200 but got 201")
    );
    assert!(output.tests.last().unwrap().skipped);
}

#[test]
fn legacy_api_and_libraries() {
    let output = test(
        r#"
        tests["Status code is 201"] = responseCode.code === 201;
        tests["Body has Ann"] = responseBody.has ? false : responseBody.indexOf("Ann") > -1;
        var data = JSON.parse(responseBody);
        postman.setEnvironmentVariable("userId", data.id);
        pm.test("lodash", () => pm.expect(_.map([1, 2], (n) => n * 2)).to.eql([2, 4]));
        pm.test("moment", () => pm.expect(require("moment")("2024-01-02").format("YYYY")).to.equal("2024"));
        pm.test("crypto", () => pm.expect(CryptoJS.MD5("a").toString()).to.equal("0cc175b9c0f1b6a831c399e269772661"));
        pm.test("hmac", () => pm.expect(require("crypto-js").HmacSHA256("m", "k").toString(CryptoJS.enc.Base64)).to.be.a("string"));
        pm.test("tv4", () => pm.expect(tv4.validate(data, { type: "object" })).to.be.true);
        pm.test("base64", () => pm.expect(atob(btoa("hi"))).to.equal("hi"));
        pm.test("uuid", () => pm.expect(require("uuid").v4()).to.match(/^[0-9a-f-]{36}$/));
        pm.test("xml", () => pm.expect(xml2Json('<a x="1"><b>t</b><b>u</b></a>')).to.eql({ a: { $: { x: "1" }, b: ["t", "u"] } }));
        "#,
    );
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    assert!(output.tests.iter().all(|t| t.passed), "{:?}", output.tests);
    assert_eq!(output.tests.len(), 10);
    assert_eq!(
        output.vars.environment.get("userId").map(String::as_str),
        Some("7")
    );
}

#[test]
fn variables_and_scopes() {
    let output = test(
        r#"
        pm.test("reads every scope", () => {
            pm.expect(pm.variables.get("baseUrl")).to.equal("http://localhost:3000");
            pm.expect(pm.variables.get("retries")).to.equal("3");
            pm.expect(pm.variables.get("who")).to.equal("globals");
            pm.expect(pm.environment.name).to.equal("Local");
            pm.expect(pm.variables.replaceIn("{{baseUrl}}/x")).to.equal("http://localhost:3000/x");
        });
        pm.environment.set("token", "t1");
        pm.collectionVariables.set("count", 5);
        pm.globals.unset("who");
        pm.variables.set("onlyNow", { a: 1 });
        pm.test("local wins", () => {
            pm.environment.set("who", "env");
            pm.expect(pm.variables.get("who")).to.equal("env");
        });
        pm.vault.get("apiKey").then((v) => pm.test("vault", () => pm.expect(v).to.equal("s3cret")));
        "#,
    );
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    assert!(output.tests.iter().all(|t| t.passed), "{:?}", output.tests);
    assert_eq!(output.tests.len(), 3);
    assert_eq!(output.vars.environment["token"], "t1");
    assert_eq!(output.vars.collection["count"], "5");
    assert!(!output.vars.globals.contains_key("who"));
    assert_eq!(output.vars.local["onlyNow"], r#"{"a":1}"#);
}

#[test]
fn pre_request_scripts_change_the_request() {
    let scripts = [
        Script { label: "collection Users".into(), source: "pm.request.headers.add({ key: 'X-Trace', value: '1' }); pm.variables.set('stamp', 'abc');".into() },
        Script { label: "folder Admin".into(), source: "".into() },
        Script {
            label: "request".into(),
            source: r#"
                pm.request.headers.upsert({ key: "accept", value: "text/plain" });
                pm.request.url.query.add({ key: "sig", value: pm.variables.get("stamp") });
                const body = JSON.parse(pm.request.body.raw);
                body.extra = true;
                pm.request.body.update(JSON.stringify(body));
                console.log("sending", { to: pm.request.url.getHost() });
            "#
            .into(),
        },
    ];
    let input = input(None);
    let output = run(Event::Prerequest, &scripts, &input, None, DEFAULT_TIMEOUT);
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    let mut file = RequestFile {
        method: "POST".into(),
        url: "{{baseUrl}}/users?page=2".into(),
        headers: vec![KeyValue {
            name: "Accept".into(),
            value: "application/json".into(),
            enabled: true,
        }],
        body: Body::Json {
            content: r#"{"name":"Ann"}"#.into(),
        },
        ..Default::default()
    };
    apply_request(&mut file, &input.request, &output.request);
    assert_eq!(file.url, "{{baseUrl}}/users?page=2&sig=abc");
    let headers: Vec<(&str, &str)> = file
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    assert_eq!(headers, [("accept", "text/plain"), ("X-Trace", "1")]);
    assert_eq!(
        file.body,
        Body::Json {
            content: r#"{"name":"Ann","extra":true}"#.into()
        }
    );
    assert_eq!(
        output.console,
        [LogLine {
            level: "log".into(),
            text: "sending {\n  \"to\": \"{{baseUrl}}\"\n}".into()
        }]
    );
}

#[test]
fn unchanged_requests_stay_as_they_were() {
    let input = input(None);
    let output = run(
        Event::Prerequest,
        &[Script {
            label: "request".into(),
            source: "pm.request.url.toString(); pm.request.headers.get('accept');".into(),
        }],
        &input,
        None,
        DEFAULT_TIMEOUT,
    );
    let original = RequestFile {
        url: "{{baseUrl}}/users?page=2".into(),
        body: Body::Json {
            content: r#"{"name":"Ann"}"#.into(),
        },
        ..Default::default()
    };
    let mut file = original.clone();
    apply_request(&mut file, &input.request, &output.request);
    assert_eq!(file.url, original.url);
    assert_eq!(file.body, original.body);
}

#[test]
fn execution_control_and_visualizer() {
    let output = run(
        Event::Prerequest,
        &[Script {
            label: "request".into(),
            source: "pm.execution.skipRequest(); postman.setNextRequest('Login');".into(),
        }],
        &input(None),
        None,
        DEFAULT_TIMEOUT,
    );
    assert!(output.skip_request);
    assert_eq!(output.next_request, Some(Some("Login".into())));
    let output = test(
        "pm.execution.setNextRequest(null); pm.visualizer.set('<b>{{name}}</b>', { name: pm.response.json().name });",
    );
    assert_eq!(output.next_request, Some(None));
    assert_eq!(
        output.visualizer,
        Some(json!({ "template": "<b>{{name}}</b>", "data": { "name": "Ann" }, "options": {} }))
    );
    assert_eq!(test("1").next_request, None);
}

#[test]
fn async_tests_and_timers() {
    let output = test(
        r#"
        setTimeout(() => pm.test("after a timer", () => pm.expect(1).to.equal(1)), 20);
        pm.test("async", async () => {
            await new Promise((resolve) => setTimeout(resolve, 10));
            pm.expect(true).to.equal(false);
        });
        await new Promise((r) => setTimeout(r, 5));
        pm.test("after await", () => {});
        "#,
    );
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    assert_eq!(
        summary(&output),
        [
            ("async".into(), false),
            ("after await".into(), true),
            ("after a timer".into(), true)
        ]
    );
}

#[test]
fn errors_and_limits() {
    let output = test("pm.test('ok', () => {}); undefinedFunction();");
    assert_eq!(
        output.errors,
        [ScriptError {
            script: "request".into(),
            message: "ReferenceError: undefinedFunction is not defined".into()
        }]
    );
    assert_eq!(output.tests.len(), 1);

    let output = test("this is not javascript");
    assert_eq!(output.errors.len(), 1);
    assert!(
        output.errors[0].message.starts_with("SyntaxError"),
        "{:?}",
        output.errors
    );

    let output = test("require('cheerio')");
    assert!(
        output.errors[0]
            .message
            .contains("Cannot find module 'cheerio'"),
        "{:?}",
        output.errors
    );

    let started = Instant::now();
    let output = run(
        Event::Test,
        &[Script {
            label: "request".into(),
            source: "pm.test('before', () => {}); console.log('spinning'); for (;;) {}".into(),
        }],
        &input(Some(response())),
        None,
        Duration::from_millis(300),
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(output.errors.len(), 1);
    assert!(
        output.errors[0].message.contains("took longer than"),
        "{:?}",
        output.errors
    );
    // What the script did before it was stopped is kept.
    assert_eq!(output.tests.len(), 1);
    assert_eq!(output.console.len(), 1);
}

#[test]
fn send_request_respects_allowed_hosts() {
    let allow: AllowHost = Arc::new(|host| host == "localhost");
    let output = run(
        Event::Test,
        &[Script {
            label: "request".into(),
            source: r#"
                pm.sendRequest("https://example.com/x", (err, res) => {
                    pm.test("blocked", () => pm.expect(err.message).to.include("not an allowed host"));
                });
                try { await pm.sendRequest({ url: "https://example.com/y", method: "POST" }); }
                catch (e) { pm.test("blocked too", () => pm.expect(e.message).to.include("example.com")); }
            "#
            .into(),
        }],
        &input(Some(response())),
        Some(allow),
        DEFAULT_TIMEOUT,
    );
    assert!(output.errors.is_empty(), "{:?}", output.errors);
    let mut s = summary(&output);
    s.sort();
    assert_eq!(s, [("blocked".into(), true), ("blocked too".into(), true)]);
}

#[test]
fn syntax_errors_are_found_without_running() {
    assert_eq!(
        syntax_error("pm.test('a', () => {});\nwhile (true) {}"),
        None
    );
    let (message, _) = syntax_error("let x = ;").unwrap();
    assert!(message.starts_with("SyntaxError"), "{message}");
    assert_eq!(syntax_error(""), None);
    assert_eq!(
        syntax_error("pm.test('a');\nconst a = 1;\nlet x = ;\n")
            .unwrap()
            .1,
        Some(3)
    );
}
