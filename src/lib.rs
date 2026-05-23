use clap::{Parser, ValueEnum};
use reqwest::Url;
use reqwest::blocking::Client;
use scraper::{ElementRef, Html, Selector, node::Node};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
#[derive(Debug, Parser)]
#[command(name = "sitesaver")]
#[command(about = "Save readable web page content as Markdown")]
pub struct Cli {
    pub url: String,
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub stdout: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Markdown)]
    pub format: OutputFormat,
    #[arg(long, value_enum, default_value_t = ExtractionMode::Main)]
    pub mode: ExtractionMode,
    #[arg(long, default_value_t = 15)]
    pub timeout: u64,
    #[arg(long)]
    pub max_bytes: Option<u64>,
    #[arg(long, default_value = "sitesaver/0.1")]
    pub user_agent: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Markdown,
    Html,
    Text,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ExtractionMode {
    Main,
    Body,
}
#[derive(Debug, Eq, PartialEq)]
pub struct ExtractedPage {
    pub title: Option<String>,
    pub content: String,
}
/// Runs the CLI workflow from fetching through writing or printing content.
pub fn run(cli: Cli) -> Result<(), String> {
    const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;
    let base_url = Url::parse(&cli.url).map_err(|error| format!("invalid URL: {error}"))?;
    let html = fetch_html(
        &base_url,
        Duration::from_secs(cli.timeout),
        cli.max_bytes.unwrap_or(DEFAULT_MAX_BYTES),
        &cli.user_agent,
    )?;
    let extracted = extract_content(&html, &base_url, cli.mode, cli.format);
    if cli.stdout {
        println!("{}", extracted.content);
        return Ok(());
    }
    const FALLBACK_FILE_NAME: &str = "site";
    let output_path = output_path(
        cli.output.as_deref(),
        extracted.title.as_deref(),
        &base_url,
        cli.format,
        FALLBACK_FILE_NAME,
    )?;
    fs::write(&output_path, extracted.content)
        .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
    println!("{}", output_path.display());
    Ok(())
}
/// Extracts the selected readable content from an HTML document.
pub fn extract_content(
    html: &str,
    base_url: &Url,
    mode: ExtractionMode,
    format: OutputFormat,
) -> ExtractedPage {
    let document = Html::parse_document(html);
    let title = page_title(&document);
    let candidate = content_candidate(&document, mode);
    let content = match format {
        OutputFormat::Markdown => render_markdown(candidate, base_url),
        OutputFormat::Html => render_html(candidate, base_url),
        OutputFormat::Text => clean_text(&candidate.text().collect::<Vec<_>>().join(" ")),
    };
    ExtractedPage { title, content }
}
/// Builds a non-conflicting output path from the requested destination and page metadata.
pub fn output_path(
    output: Option<&Path>,
    title: Option<&str>,
    url: &Url,
    format: OutputFormat,
    fallback_file_name: &str,
) -> Result<PathBuf, String> {
    const CURRENT_DIRECTORY: &str = ".";
    let extension = match format {
        OutputFormat::Markdown => "md",
        OutputFormat::Html => "html",
        OutputFormat::Text => "txt",
    };
    let requested = output.unwrap_or_else(|| Path::new(CURRENT_DIRECTORY));
    let path = if requested.extension().is_some() && !requested.is_dir() {
        requested.to_path_buf()
    } else {
        requested.join(format!(
            "{}.{extension}",
            file_stem(title, url, fallback_file_name)
        ))
    };
    unique_path(&path)
}
/// Fetches a URL body as UTF-8 while enforcing timeout and response size limits.
fn fetch_html(
    url: &Url,
    timeout: Duration,
    max_bytes: u64,
    user_agent: &str,
) -> Result<String, String> {
    let client = Client::builder()
        .timeout(timeout)
        .user_agent(user_agent)
        .build()
        .map_err(|error| format!("failed to build HTTP client: {error}"))?;
    let mut response = client
        .get(url.clone())
        .send()
        .map_err(|error| format!("failed to fetch {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("request failed for {url}: {error}"))?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read response body: {error}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("response exceeds max bytes: {max_bytes}"));
    }
    String::from_utf8(bytes).map_err(|error| format!("response is not UTF-8: {error}"))
}
/// Reads and normalizes the document title when it is present.
fn page_title(document: &Html) -> Option<String> {
    const TITLE_SELECTOR: &str = "title";
    document
        .select(&selector(TITLE_SELECTOR))
        .next()
        .map(|title| clean_text(&title.text().collect::<Vec<_>>().join(" ")))
        .filter(|title| !title.is_empty())
}
/// Chooses the DOM subtree that should be converted into output content.
fn content_candidate(document: &Html, mode: ExtractionMode) -> ElementRef<'_> {
    const BODY_SELECTOR: &str = "body";
    const MAIN_SELECTORS: [&str; 3] = ["main", "article", "[role=\"main\"]"];
    match mode {
        ExtractionMode::Main => MAIN_SELECTORS
            .iter()
            .filter_map(|query| document.select(&selector(query)).next())
            .next()
            .or_else(|| document.select(&selector(BODY_SELECTOR)).next())
            .unwrap_or_else(|| document.root_element()),
        ExtractionMode::Body => document
            .select(&selector(BODY_SELECTOR))
            .next()
            .unwrap_or_else(|| document.root_element()),
    }
}
/// Renders a cleaned DOM subtree back into HTML.
fn render_html(element: ElementRef<'_>, base_url: &Url) -> String {
    render_children(element, base_url, HtmlRenderMode::Html)
        .trim()
        .to_string()
}
/// Renders a cleaned DOM subtree into normalized Markdown.
fn render_markdown(element: ElementRef<'_>, base_url: &Url) -> String {
    normalize_markdown(&render_children(
        element,
        base_url,
        HtmlRenderMode::Markdown,
    ))
}
/// Renders child nodes while skipping elements classified as page noise.
fn render_children(element: ElementRef<'_>, base_url: &Url, mode: HtmlRenderMode) -> String {
    element
        .children()
        .map(|child| match child.value() {
            Node::Text(text) => match mode {
                HtmlRenderMode::Markdown => clean_text(text),
                HtmlRenderMode::Html => escape_html(text),
            },
            Node::Element(_) => ElementRef::wrap(child)
                .filter(|child_element| !is_noise_element(child_element))
                .map(|child_element| render_element(child_element, base_url, mode))
                .unwrap_or_default(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("")
}
/// Dispatches element rendering to the selected output format.
fn render_element(element: ElementRef<'_>, base_url: &Url, mode: HtmlRenderMode) -> String {
    match mode {
        HtmlRenderMode::Html => render_html_element(element, base_url),
        HtmlRenderMode::Markdown => render_markdown_element(element, base_url),
    }
}
/// Renders a single element as HTML and rewrites URL-like attributes.
fn render_html_element(element: ElementRef<'_>, base_url: &Url) -> String {
    let tag = element.value().name();
    let children = render_children(element, base_url, HtmlRenderMode::Html);
    let attrs = element
        .value()
        .attrs()
        .filter_map(|(name, value)| {
            if name == "href" || name == "src" {
                absolutize_url(base_url, value)
                    .map(|url| format!(" {name}=\"{}\"", escape_html(&url)))
            } else {
                Some(format!(" {name}=\"{}\"", escape_html(value)))
            }
        })
        .collect::<Vec<_>>()
        .join("");
    format!("<{tag}{attrs}>{children}</{tag}>")
}
/// Renders a single element as Markdown using simple tag-specific rules.
fn render_markdown_element(element: ElementRef<'_>, base_url: &Url) -> String {
    let tag = element.value().name();
    let children = render_children(element, base_url, HtmlRenderMode::Markdown);
    let text = clean_text(&children);
    match tag {
        "h1" => block(&format!("# {text}")),
        "h2" => block(&format!("## {text}")),
        "h3" => block(&format!("### {text}")),
        "h4" => block(&format!("#### {text}")),
        "h5" => block(&format!("##### {text}")),
        "h6" => block(&format!("###### {text}")),
        "p" => block(&text),
        "br" => "\n".to_string(),
        "strong" | "b" => format!("**{text}**"),
        "em" | "i" => format!("_{text}_"),
        "code" => format!("`{}`", text.replace('`', "\\`")),
        "pre" => block(&format!("```\n{}\n```", children.trim())),
        "blockquote" => block(
            &text
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "ul" | "ol" => block(&children),
        "li" => format!("- {}\n", text),
        "a" => link_markdown(element, base_url, &text),
        "table" => block(&render_table(element, base_url)),
        "thead" | "tbody" | "tr" | "th" | "td" => children,
        "div" | "section" | "main" | "article" | "body" => block(&children),
        _ => children,
    }
}
/// Renders an anchor as Markdown when it has visible text and a valid URL.
fn link_markdown(element: ElementRef<'_>, base_url: &Url, text: &str) -> String {
    element
        .value()
        .attr("href")
        .and_then(|href| absolutize_url(base_url, href))
        .filter(|_| !text.is_empty())
        .map(|url| format!("[{text}]({url})"))
        .unwrap_or_else(|| text.to_string())
}
/// Converts an HTML table into a basic Markdown table.
fn render_table(element: ElementRef<'_>, base_url: &Url) -> String {
    let rows = element
        .select(&selector("tr"))
        .map(|row| {
            row.children()
                .filter_map(ElementRef::wrap)
                .filter(|cell| matches!(cell.value().name(), "th" | "td"))
                .map(|cell| clean_text(&render_children(cell, base_url, HtmlRenderMode::Markdown)))
                .collect::<Vec<_>>()
        })
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>();
    match rows.split_first() {
        Some((header, body)) => {
            let header_line = table_row(header);
            let separator =
                table_row(&header.iter().map(|_| "---".to_string()).collect::<Vec<_>>());
            let body_lines = body
                .iter()
                .map(|row| table_row(row))
                .collect::<Vec<_>>()
                .join("\n");
            [header_line, separator, body_lines]
                .into_iter()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        None => String::new(),
    }
}
/// Detects tags and attributes that usually represent non-content page chrome.
fn is_noise_element(element: &ElementRef<'_>) -> bool {
    const NOISE_TAGS: [&str; 12] = [
        "script", "style", "noscript", "template", "svg", "canvas", "iframe", "head", "meta",
        "link", "nav", "footer",
    ];
    const NOISE_KEYWORDS: [&str; 13] = [
        "ad", "ads", "advert", "banner", "cookie", "footer", "header", "menu", "modal", "nav",
        "promo", "share", "social",
    ];
    let tag = element.value().name();
    let attr_text = ["id", "class", "role", "aria-label"]
        .iter()
        .filter_map(|name| element.value().attr(name))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    NOISE_TAGS.contains(&tag)
        || NOISE_KEYWORDS
            .iter()
            .any(|keyword| attr_text.contains(keyword))
}
/// Chooses a safe file stem from the title, URL path, or fallback name.
fn file_stem(title: Option<&str>, url: &Url, fallback_file_name: &str) -> String {
    title
        .map(slugify)
        .filter(|title| !title.is_empty())
        .or_else(|| {
            url.path_segments()
                .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
                .map(slugify)
                .filter(|path| !path.is_empty())
        })
        .unwrap_or_else(|| fallback_file_name.to_string())
}
/// Adds a numeric suffix when the requested output path already exists.
fn unique_path(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("invalid output file name: {}", path.display()))?;
    let extension = path.extension().and_then(|extension| extension.to_str());
    (2..)
        .map(|index| {
            let file_name = match extension {
                Some(extension) => format!("{stem}-{index}.{extension}"),
                None => format!("{stem}-{index}"),
            };
            parent.join(file_name)
        })
        .find(|candidate| !candidate.exists())
        .ok_or_else(|| "failed to create unique output path".to_string())
}
/// Converts arbitrary text into a lowercase ASCII file-name stem.
fn slugify(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
/// Collapses repeated blank lines and trims the final Markdown output.
fn normalize_markdown(markdown: &str) -> String {
    markdown
        .lines()
        .map(str::trim_end)
        .scan(false, |previous_blank, line| {
            let is_blank = line.is_empty();
            let keep = !is_blank || !*previous_blank;
            *previous_blank = is_blank;
            Some(keep.then_some(line))
        })
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}
/// Collapses whitespace in text extracted from HTML nodes.
fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// Wraps non-empty Markdown fragments as block-level content.
fn block(value: &str) -> String {
    match value.trim() {
        "" => String::new(),
        trimmed => format!("\n\n{trimmed}\n\n"),
    }
}
/// Renders a Markdown table row from already-normalized cell values.
fn table_row(values: &[String]) -> String {
    format!("| {} |", values.join(" | "))
}
/// Resolves a possibly relative URL against the source page URL.
fn absolutize_url(base_url: &Url, value: &str) -> Option<String> {
    base_url.join(value).ok().map(|url| url.to_string())
}
/// Parses a static CSS selector used by the extraction pipeline.
fn selector(query: &str) -> Selector {
    Selector::parse(query).expect("static CSS selector should be valid")
}
/// Escapes text for safe inclusion in generated HTML.
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[derive(Clone, Copy)]
enum HtmlRenderMode {
    Markdown,
    Html,
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    /// Creates the shared base URL used by extraction tests.
    fn base_url() -> Url {
        Url::parse("https://example.com/docs/page.html").unwrap()
    }
    /// Creates an isolated temporary directory for file path tests.
    fn temp_directory(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("sitesaver-test-{name}-{suffix}"));
        fs::create_dir_all(&directory).unwrap();
        directory
    }
    #[test]
    /// Verifies that non-content document elements are removed before Markdown rendering.
    fn removes_script_head_meta_and_style_from_markdown() {
        // GIVEN
        let html = r#"
            <html>
                <head><title>Ignored</title><meta name="x"><style>.x{}</style></head>
                <body><main><script>bad()</script><p>Hello content</p></main></body>
            </html>
        "#;
        // WHEN
        let actual = extract_content(
            html,
            &base_url(),
            ExtractionMode::Main,
            OutputFormat::Markdown,
        );
        // THEN
        let expected = "Hello content";
        assert_eq!(actual.content, expected);
    }
    #[test]
    /// Verifies that main content is preferred over surrounding body content.
    fn prefers_main_when_main_exists() {
        // GIVEN
        let html = r#"
            <html><body><p>Body text</p><main><h1>Main title</h1><p>Main text</p></main></body></html>
        "#;
        // WHEN
        let actual = extract_content(
            html,
            &base_url(),
            ExtractionMode::Main,
            OutputFormat::Markdown,
        );
        // THEN
        let expected = "# Main title\n\nMain text";
        assert_eq!(actual.content, expected);
    }
    #[test]
    /// Verifies that body content is used when no main candidate exists.
    fn uses_cleaned_body_when_main_is_missing() {
        // GIVEN
        let html = r#"
            <html><body><nav>Navigation</nav><p>Body text</p><footer>Footer</footer></body></html>
        "#;
        // WHEN
        let actual = extract_content(
            html,
            &base_url(),
            ExtractionMode::Main,
            OutputFormat::Markdown,
        );
        // THEN
        let expected = "Body text";
        assert_eq!(actual.content, expected);
    }
    #[test]
    /// Verifies that Markdown links are resolved against the source URL.
    fn converts_relative_links_to_absolute_links() {
        // GIVEN
        let html = r#"
            <html><body><main><p><a href="../guide">Guide</a></p></main></body></html>
        "#;
        // WHEN
        let actual = extract_content(
            html,
            &base_url(),
            ExtractionMode::Main,
            OutputFormat::Markdown,
        );
        // THEN
        let expected = "[Guide](https://example.com/guide)";
        assert_eq!(actual.content, expected);
    }
    #[test]
    /// Verifies that page titles are used to generate Markdown file names.
    fn generates_output_path_from_title() {
        // GIVEN
        let directory = temp_directory("title");
        let url = base_url();
        // WHEN
        let actual = output_path(
            Some(&directory),
            Some("Example Page!"),
            &url,
            OutputFormat::Markdown,
            "site",
        )
        .unwrap();
        // THEN
        let expected = directory.join("example-page.md");
        assert_eq!(actual, expected);
    }
    #[test]
    /// Verifies that URL paths are used when no page title is available.
    fn generates_output_path_from_url_when_title_is_missing() {
        // GIVEN
        let directory = temp_directory("url");
        let url = base_url();
        // WHEN
        let actual =
            output_path(Some(&directory), None, &url, OutputFormat::Markdown, "site").unwrap();
        // THEN
        let expected = directory.join("page-html.md");
        assert_eq!(actual, expected);
    }
    #[test]
    /// Verifies that existing output files are preserved with numeric suffixes.
    fn generates_numbered_output_path_when_file_exists() {
        // GIVEN
        let directory = temp_directory("numbered");
        let existing = directory.join("example-page.md");
        fs::write(&existing, "already exists").unwrap();
        let url = base_url();
        // WHEN
        let actual = output_path(
            Some(&directory),
            Some("Example Page"),
            &url,
            OutputFormat::Markdown,
            "site",
        )
        .unwrap();
        // THEN
        let expected = directory.join("example-page-2.md");
        assert_eq!(actual, expected);
    }
}
