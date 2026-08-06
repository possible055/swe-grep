use serde_json::json;
use std::fs;
use std::hint::black_box;
use std::time::{Duration, Instant};
use swe_grep::core::get_repo_map;
use swe_grep::executor::{InstantContextToolCall, ToolExecutor};
use swe_grep::path_filter::PathFilterConfig;
use tempfile::TempDir;

const WARMUP_RUNS: usize = 3;
const SAMPLE_RUNS: usize = 25;

fn main() {
    let fixture = BenchmarkFixture::new();
    let executor = ToolExecutor::new(fixture.root());

    let readfile = json!({"type": "readfile", "file": "/codebase/large.txt"});
    run_case("large readfile", || executor.exec_command(&readfile));

    let tree = json!({"type": "tree", "path": "/codebase", "levels": 6});
    run_case("deep tree", || executor.exec_command(&tree));

    let glob = json!({
        "type": "glob",
        "pattern": "**/*.rs",
        "path": "/codebase",
        "type_filter": "file"
    });
    run_case("recursive glob", || executor.exec_command(&glob));

    run_case("RepoMap fallback", || {
        get_repo_map(fixture.root(), 4, &PathFilterConfig::default()).tree
    });

    let rg = json!({
        "type": "rg",
        "pattern": "needle|fn item",
        "path": "/codebase",
        "include": ["*.rs", "*.txt", "src/**/*.rs", "deep/**/*.rs", "small/**/*.rs"]
    });
    let rg_probe = executor.exec_command(&rg);
    if rg_probe.contains("ripgrep ('rg') is required") {
        println!("broad rg: skipped (rg unavailable)");
    } else {
        run_case("broad rg", || executor.exec_command(&rg));
    }

    let calls = (0..8)
        .map(|index| InstantContextToolCall {
            id: format!("bench-{index}"),
            name: format!("command{}", index + 1),
            args: if index % 2 == 0 {
                json!({"type": "glob", "pattern": "**/*.rs", "path": "/codebase"})
            } else {
                json!({"type": "readfile", "file": "/codebase/large.txt", "end_line": 20})
            },
        })
        .collect::<Vec<_>>();
    run_case("eight-command batch", || {
        let updates = executor.exec_restricted_exec_step("bench", &calls, 5_000);
        updates
            .iter()
            .map(|update| update.output.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    });
}

fn run_case(name: &str, mut operation: impl FnMut() -> String) {
    for _ in 0..WARMUP_RUNS {
        black_box(operation());
    }

    let mut durations = Vec::with_capacity(SAMPLE_RUNS);
    let mut output_bytes = 0_usize;
    for _ in 0..SAMPLE_RUNS {
        let started = Instant::now();
        let output = black_box(operation());
        durations.push(started.elapsed());
        output_bytes = output.len();
        black_box(output);
    }
    durations.sort_unstable();
    let p50 = percentile(&durations, 50);
    let p95 = percentile(&durations, 95);
    println!(
        "{name}: p50={:.3}ms p95={:.3}ms output={}B",
        milliseconds(p50),
        milliseconds(p95),
        output_bytes
    );
}

fn percentile(durations: &[Duration], percentile: usize) -> Duration {
    let index = (durations.len() - 1) * percentile / 100;
    durations[index]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

struct BenchmarkFixture {
    temp: TempDir,
}

impl BenchmarkFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("create benchmark fixture");
        let root = temp.path();
        let small = root.join("small");
        fs::create_dir(&small).expect("create small fixture directory");
        for index in 0..1_000 {
            fs::write(
                small.join(format!("item_{index:04}.rs")),
                format!("fn item_{index}() {{ /* needle */ }}\n"),
            )
            .expect("write small fixture file");
        }

        let mut deep = root.join("deep");
        for depth in 0..8 {
            deep = deep.join(format!("level_{depth}"));
            fs::create_dir_all(&deep).expect("create deep fixture directory");
            fs::write(deep.join(format!("depth_{depth}.rs")), "fn needle() {}\n")
                .expect("write deep fixture file");
        }

        let mut large = String::with_capacity(4_000_100);
        large.push_str(&"x".repeat(4_000_000));
        large.push_str("\nneedle\n");
        fs::write(root.join("large.txt"), large).expect("write large fixture file");
        Self { temp }
    }

    fn root(&self) -> &std::path::Path {
        self.temp.path()
    }
}
