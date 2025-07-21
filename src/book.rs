use log::{debug, info, warn};
use serde::{Deserialize};
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use serde::Serialize;

use crate::GenericError;

/// Represents basic information about a book, including its title and ISBN.
#[derive(Debug, Clone)]
pub struct BookInfo {
    /// The title of the book.
    pub title: String,
    /// The normalized ISBN (ISBN-10 or ISBN-13 without hyphens).
    pub isbn: String,
}

impl BookInfo {
    /// Creates a new `BookInfo` instance, normalizing the provided ISBN.
    ///
    /// This constructor attempts to normalize the `isbn` string by removing hyphens
    /// and validating its format.
    ///
    /// # Arguments
    ///
    /// * `title` - The title of the book.
    /// * `isbn` - The raw ISBN string, which may contain hyphens.
    ///
    /// # Returns
    ///
    /// Returns `Ok(BookInfo)` if the ISBN is successfully normalized and
    /// a `BookInfo` instance is created. Returns `Err(String)` if the ISBN
    /// is invalid.
    ///
    /// # Errors
    ///
    /// Returns an `Err` message if the `isbn` string is:
    /// - Not ASCII.
    /// - Of an invalid length (less than 10 or greater than 13 characters after normalization).
    /// - Contains invalid characters (e.g., non-digits in the prefix, or an invalid last character).
    pub fn new(title: &str, isbn: &str) -> Result<BookInfo, String> {
        let norm_isbn = normalize_isbn(isbn)?;
        Ok(BookInfo {
            title: title.to_string(),
            isbn: norm_isbn,
        })
    }
}

/// Normalize ISBN string by removing hyphens and validating the format.
///
/// This function takes a raw ISBN string, removes any hyphens, and then
/// performs basic validation checks on its length and character content.
///
/// # Arguments
///
/// * `isbn_str` - The ISBN string to normalize. This can be either ISBN-10 or ISBN-13
///   and may contain hyphens.
///
/// # Returns
///
/// Returns `Ok(String)` containing the normalized ISBN string (without hyphens)
/// if the input is valid. Returns `Err(String)` with an error message if the
/// ISBN format is invalid.
///
/// # Errors
///
/// Returns an `Err` message if the `isbn_str` is:
/// - Not entirely composed of ASCII characters.
/// - Of an invalid length (after removing hyphens, must be between 10 and 13 inclusive).
/// - Contains non-digit characters in its prefix (all characters except the last one).
/// - Has an invalid last character (must be a digit, 'x', or 'X').
pub fn normalize_isbn(isbn_str: &str) -> Result<String, String> {
    let mut norm = isbn_str.to_string();
    norm.retain(|c| c != '-');

    if !norm.is_ascii() {
        return Err(format!("Not ascii code"));
    }

    let norm_length = norm.len();
    if norm_length < 10 || norm_length > 13 {
        return Err(format!("Invalid legth of string"));
    }

    if let Err(_) = norm.as_str()[0..norm_length - 1].parse::<u64>() {
        return Err(format!("Invalid charactor(prefix)"));
    }

    // last charcter is 'x' or 'X' or a digit.
    let is_valid_char = match isbn_str.chars().last() {
        Some('x') => true,
        Some('X') => true,
        Some(c) => c.is_ascii_digit(),
        None => false,
    };
    if !is_valid_char {
        return Err(format!("Invalid charactor(suffix)"));
    }

    return Ok(norm);
}

#[test]
fn test_normalize_isbn() {
    let length_zero = "";
    assert!(normalize_isbn(length_zero).is_err());

    let length_short = "12345";
    assert!(normalize_isbn(length_short).is_err());

    let length_long = "12345678901234";
    assert!(normalize_isbn(length_long).is_err());

    let invalid_character = "abc1234567890";
    assert!(normalize_isbn(invalid_character).is_err());

    let isdn10 = "1234567890";
    assert!(normalize_isbn(isdn10).is_ok());

    let isdn10_with_hyphen = "4-567890-12-3";
    assert!(normalize_isbn(isdn10_with_hyphen).is_ok());

    let isdn13 = "1234567890123";
    assert!(normalize_isbn(isdn13).is_ok());

    let isdn13_with_hyphen = "123-4-567890-12-3";
    assert!(normalize_isbn(isdn13_with_hyphen).is_ok());

    let isdn13_with_x = "123456789012x";
    assert!(normalize_isbn(isdn13_with_x).is_ok());

    let isdn13_with_large_x = "123456789012X";
    assert!(normalize_isbn(isdn13_with_large_x).is_ok());
}

/// Searches for books by title using the CiNii Books API.
///
/// This function scrapes the CiNii Books website to find books matching the given title.
/// It extracts book titles and their associated ISBNs.
///
/// # Arguments
///
/// * `title` - The exact title of the book to search for.
///
/// # Returns
///
/// Returns a `Result` containing a `Vec<BookInfo>` on success, where each `BookInfo`
/// represents a found book with its title and normalized ISBN. Returns `GenericError`
/// on failure, typically due to network issues or parsing errors.
///
/// # Errors
///
/// Returns `GenericError` if:
/// - The HTTP request fails (e.g., network error, DNS resolution).
/// - The response body cannot be retrieved or parsed.
/// - The HTML parsing with `scraper` fails unexpectedly.
///
/// # Examples
///
/// ```no_run
/// let title = "アルゴリズム";
/// let books = search_book_c(title).await?;
/// for book in books {
///     println!("Title: {}, ISBN: {}", book.title, book.isbn);
/// }
/// ```
pub async fn search_book_c(title: &str) -> Result<Vec<BookInfo>, GenericError> {
    let host = "https://ci.nii.ac.jp";
    let api = "/books/search?advanced=true&count=20&sortorder=1&type=1&title_exact=true&update_keep=true&title=";
    let search_url = format!("{}{}{}", host, api, title);

    let body = reqwest::get(search_url).await?.text().await?;
    debug!("body: {}", body);

    let selector = scraper::Selector::parse("dt.item_mainTitle a").unwrap();
    let document = scraper::Html::parse_document(&body);
    let titles = document.select(&selector);

    let mut result = Vec::new();
    for t in titles {
        let book_title = t.text().collect::<String>();
        if !book_title.starts_with(title) {
            debug!("{} is not {}", book_title, title);
            continue;
        }
        let book_url = t.value().attr("href").unwrap();
        let search_url = format!("{}{}", host, book_url);
        let body = reqwest::get(search_url).await?.text().await?;
        debug!("body: {}", body);
        let selector = scraper::Selector::parse("li.isbn li").unwrap();
        let document = scraper::Html::parse_document(&body);
        let isbns = document.select(&selector);
        for isbn_elm in isbns {
            let isbn = isbn_elm.text().collect::<String>();
            if let Ok(book_info) = BookInfo::new(&book_title, &isbn) {
                result.push(book_info);
            }
        }
    }

    Ok(result)
}

#[tokio::test]
async fn test_search_book_c() {
    let title = "アルゴリズム";
    let books = match search_book_c(title).await {
        Ok(books) => books,
        Err(why) => panic!("{:?}", why),
    };
    assert!(books.len() > 0);
}

/// Searches for books by title using the NDL Search API (National Diet Library).
///
/// This function scrapes the NDL Search website to find books matching the given title.
/// It extracts book titles and their associated ISBNs.
///
/// # Arguments
///
/// * `title` - The title of the book to search for.
///
/// # Returns
///
/// Returns a `Result` containing a `Vec<BookInfo>` on success, where each `BookInfo`
/// represents a found book with its title and normalized ISBN. Returns `GenericError`
/// on failure, typically due to network issues or parsing errors.
///
/// # Errors
///
/// Returns `GenericError` if:
/// - The HTTP request fails (e.g., network error, DNS resolution).
/// - The response body cannot be retrieved or parsed.
/// - The HTML parsing with `scraper` fails unexpectedly.
///
/// # Examples
///
/// ```no_run
/// let title = "アルゴリズム";
/// let books = search_book_k(title).await?;
/// for book in books {
///     println!("Title: {}, ISBN: {}", book.title, book.isbn);
/// }
/// ```
pub async fn search_book_k(title: &str) -> Result<Vec<BookInfo>, GenericError> {
    let ndlsearch = "https://ndlsearch.ndl.go.jp";
    let api = "/search?cs=bib&display=panel&from=0&size=20&f-ht=ndl&f-ht=library&f-mt=dtbook&f-doc_style=paper&q-title=";
    let search_url = format!("{}{}{}", ndlsearch, api, title);

    let body = reqwest::get(search_url).await?.text().await?;
    debug!("body: {}", body);

    let selector = scraper::Selector::parse("h3.base-heading a").unwrap();
    let document = scraper::Html::parse_document(&body);
    let titles = document.select(&selector);

    let mut result = Vec::new();
    for t in titles {
        let book_title = t.text().collect::<String>();
        if !book_title.starts_with(title) {
            println!("{} is not {}", book_title, title);
            continue;
        }
        let book_url = t.value().attr("href").unwrap();
        let search_url = format!("{}{}", ndlsearch, book_url);
        let body = reqwest::get(search_url).await?.text().await?;
        let selector = scraper::Selector::parse("div.bib-info-k00200 span").unwrap();
        let document = scraper::Html::parse_document(&body);
        let isbns = document.select(&selector);
        for isbn_elm in isbns {
            let isbn = isbn_elm.text().collect::<String>();
            if let Ok(book_info) = BookInfo::new(&book_title, &isbn) {
                result.push(book_info);
            }
        }
    }

    Ok(result)
}

#[tokio::test]
async fn test_search_book_k() {
    let title = "アルゴリズム";
    let books = match search_book_k(title).await {
        Ok(books) => books,
        Err(why) => panic!("{:?}", why),
    };
    assert!(books.len() > 0);
}

/// # Calil API Response Structures and Examples
///
/// This section defines the Rust types that map to the JSON responses from the Calil API,
/// along with detailed examples of expected JSON structures, including polling scenarios.
///
/// ## Parameters
/// The `book` field in the Calil API typically contains keys like these representing library IDs and their full names:
/// ```json
/// "book": {
///   "Tokyo_NDL": "国立国会図書館",
///   "Tokyo_Pref": "東京都立図書館"
/// },
/// ```
/// 
/// ## Normal Respose
/// 
/// An example of a successful, single-response query result:
/// ```json
/// {
///   "session": "9473cf1ab9d31b770ee891f200bc34f123c133c3e73b5ed5c5e59a6336cc534e",
///   "continue": 0,
///   "books": {
///     "978-4756102133": {
///       "Tokyo_NDL": {
///         "status": "Cache",
///         "libkey": {
///           "デジタル": "蔵書あり"
///         },
///         "reserveurl": "https://ndlsearch.ndl.go.jp/books/R100000002-I000002412296"
///       },
///       "Tokyo_Pref": {
///         "status": "Cache",
///         "libkey": {},
///         "reserveurl":
///         ""
///       }
///     }
///   }
/// }
/// ```
/// 
/// ## Response of Calil with Polling (2sec interval)
///
/// The Calil API often requires polling for results. Below are examples of how the
/// response changes during the polling process. `continue` field indicates if polling
/// should continue (1 for yes, 0 for no).
///
/// - **1st response (initial query, status "Running")** 
/// ```json
/// {
///     "session": "93ce705192130e6a2ba096e3e19929258fb1bb21638e9a6a8e083cb7884e9d65", "continue": 1,
///     "books": {
///         "9784004306726": {
///             "Tokyo_Pref": {"status": "Running", "reserveurl": ""},
///             "Tokyo_NDL": {"status": "Running", "reserveurl": ""}
///         }
///     }
/// }
/// ```
///
/// - **2nd response (Tokyo_Pref results available, Tokyo_NDL still running)**
/// ```json
/// {
///     "session": "93ce705192130e6a2ba096e3e19929258fb1bb21638e9a6a8e083cb7884e9d65", "continue": 1,
///     "books": {
///         "9784004306726": {
///             "Tokyo_Pref": {
///                 "status": "OK", "libkey": {"中央": "館内のみ", "多摩": "館内のみ"},"reserveurl": "https://catalog.library.metro.tokyo.lg.jp/winj/opac/switch-detail-iccap.do?bibid=1105160115"},
///             "Tokyo_NDL": {"status": "Running", "reserveurl": ""}
///         }
///     }
/// }
/// ```
///
/// - **15th response (Tokyo_Pref still OK, Tokyo_NDL still running after many retries)**
/// ```json
/// {
///     "session": "93ce705192130e6a2ba096e3e19929258fb1bb21638e9a6a8e083cb7884e9d65", "continue": 1,
///     "books": {
///         "9784004306726": {
///             "Tokyo_Pref": {"status": "OK", "libkey": {"中央": "館内のみ", "多摩": "館内のみ"}, "reserveurl": "https://catalog.library.metro.tokyo.lg.jp/winj/opac/switch-detail-iccap.do?bibid=1105160115"},
///             "Tokyo_NDL": {"status": "Running", "reserveurl": ""}
///         }
///     }
/// }
/// ```
///
/// - **Final response (Tokyo_NDL returns "Error", `continue` becomes 0)**
/// ```json
/// {
///     "session": "93ce705192130e6a2ba096e3e19929258fb1bb21638e9a6a8e083cb7884e9d65", "continue": 0,
///     "books": {
///         "9784004306726": {
///             "Tokyo_Pref": {"status": "OK", "libkey": {"中央": "館内のみ", "多摩": "館内のみ"}, "reserveurl": "https://catalog.library.metro.tokyo.lg.jp/winj/opac/switch-detail-iccap.do?bibid=1105160115"},
///             "Tokyo_NDL": {"status": "Error"}
///         }
///     }
/// }
/// ```
///
 
/// Alias for an ISBN-10 or ISBN-13 string, typically without hyphens.
type Isbn = String;
/// Alias for the full name of a library (e.g., "国立国会図書館").
type LibraryName = String;
/// Alias for the Calil API system ID of a library (e.g., "Tokyo_NDL").
type LibraryId = String;
/// Alias for a specific location within a library (e.g., "デジタル", "中央").
type LibraryLocation = String;
/// Alias for the availability status of a book at a specific library location (e.g., "蔵書あり", "館内のみ").
type LibraryStatus = String;
/// A map representing book availability status: ISBN -> (Library ID -> Library Details).
type BooksInLibrary = HashMap<Isbn, HashMap<LibraryId, LibraryDetails>>;

/// Represents the top-level response structure from the Calil API.
#[derive(Deserialize, Debug)]
struct CalilResponse {
    /// The session ID used for polling subsequent requests.
    session: String,
    /// A flag indicating whether further polling is required (1: continue, 0: finished).
    #[serde(rename = "continue")]
    continue_val: u32,
    /// A map of ISBNs to their availability details across various libraries.
    books: BooksInLibrary,
}

/// Represents the detailed availability status of a book at a specific library.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LibraryDetails {
    /// The overall status of the library's check for the book (e.g., "Cache", "Running", "OK", "Error").
    pub status: String,
    /// An optional map of specific library locations to their availability status for the book.
    /// This field is typically present when `status` is "OK" or "Cache".
    pub libkey: Option<HashMap<LibraryLocation, LibraryStatus>>,
    /// An optional URL to the book's details page or reservation page on the library's website.
    pub reserveurl: Option<String>,
}

/// Queries the Calil API to get the current availability status of books in specified libraries.
///
/// This function initiates a request to the Calil API for a list of ISBNs and library system IDs.
/// It handles the API's polling mechanism, waiting for results to become available if necessary.
///
/// # Arguments
///
/// * `appkey` - The API key for accessing the Calil service.
/// * `isbns` - A vector of normalized ISBN strings for the books to query.
/// * `systemids` - A vector of Calil library system IDs (e.g., "Tokyo_NDL", "Tokyo_Pref")
///   for the libraries to check.
///
/// # Returns
///
/// Returns a `Result` containing a `BooksInLibrary` map on success, which details
/// the availability status of each book in each queried library. Returns `GenericError`
/// on failure.
///
/// # Errors
///
/// Returns `GenericError` if:
/// - The HTTP request fails (`reqwest::Error`) during `send()` or `text()` operations.
/// - The JSON response from the Calil API cannot be deserialized into `CalilResponse` (`serde_json::Error`).
///
/// # Examples
///
/// ```no_run
/// let appkey = "YOUR_CALIL_API_KEY"; // Replace with your actual key
/// let isbns = vec!["9784756102133".to_string()];
/// let libraries = vec!["Tokyo_NDL".to_string(), "Tokyo_Pref".to_string()];
///
/// let book_status = query_book_status(appkey, &isbns, &libraries).await?;
/// for (isbn, system_details) in book_status {
///     println!("ISBN: {}", isbn);
///     for (system_id, details) in system_details {
///         println!("  Library {}: Status {:?}", system_id, details.status);
///     }
/// }
/// ```
pub async fn query_book_status(
    appkey: &str,
    isbns: &Vec<String>,
    systemids: &Vec<String>,
) -> Result<BooksInLibrary, GenericError> {
    let url = "https://api.calil.jp/check";
    let mut query = vec![("appkey", appkey), ("callback", "no")];
    let isbns_csv = isbns.join(",");
    query.push(("isbn", &isbns_csv));
    let systemids_csv = systemids.join(",");
    query.push(("systemid", &systemids_csv));

    let client = reqwest::Client::new();
    let body = client
        .get(url)
        .query(&query)
        .send()
        .await
        .unwrap()
        .text()
        .await?;
    debug!("body: {}", body);
    let mut calil_response = serde_json::from_str::<CalilResponse>(&body)?;
    while calil_response.continue_val == 1 {
        // Wait for the book search to complete.
        sleep(Duration::from_secs(5)).await;
        let polling_query = vec![
            ("appkey", appkey),
            ("callback", "no"),
            ("session", &calil_response.session),
        ];
        let body = client
            .get(url)
            .query(&polling_query)
            .send()
            .await
            .unwrap()
            .text()
            .await?;
        debug!("body: {}", body);
        calil_response = serde_json::from_str::<CalilResponse>(&body)?;
    }

    Ok(calil_response.books)
}

#[tokio::test]
async fn test_query_book_status() {
    let appkey = std::env::var("CALIL_APPKEY").expect("Expected a token in the environment");
    // はじめて読む486
    let isbns = vec!["9784756102133".to_string()];
    let libraries = vec!["Tokyo_NDL".to_string(), "Tokyo_Pref".to_string()];

    let book_status = query_book_status(&appkey, &isbns, &libraries)
        .await
        .unwrap();
    for (isbn, systemids) in book_status {
        assert_eq!(isbn, isbns[0]);
        for (systemid, _) in systemids {
            assert!(libraries.contains(&systemid));
        }
    }
}

/// Manages the process of searching for books and querying their availability status
/// in various libraries using the Calil API.
pub struct BookDb {
    /// The original title used for the book search query.
    pub title: String,
    /// A map of library system IDs to their human-readable names.
    pub libraries: HashMap<LibraryId, LibraryName>,
    /// A vector of `BookInfo` found during the search phase.
    books: Vec<BookInfo>,
    /// A map storing the availability status of books in libraries, retrieved from Calil.
    pub status: BooksInLibrary,
    /// The application key for the Calil API.
    appkey: String,
}

impl BookDb {
    /// Creates a new `BookDb` instance.
    ///
    /// Initializes a new `BookDb` struct with the provided Calil API key and a list
    /// of libraries to check.
    ///
    /// # Arguments
    ///
    /// * `appkey` - The Calil API key.
    /// * `libraries` - A `HashMap` where keys are Calil library system IDs (e.g., "Tokyo_NDL")
    ///   and values are their human-readable names (e.g., "国立国会図書館").
    ///
    /// # Returns
    ///
    /// A new `BookDb` instance ready for searching and status queries.
    pub fn new(appkey: &str, libraries: &HashMap<String, String>) -> BookDb {
        BookDb {
            title: "".to_string(),
            libraries: libraries.clone(),
            books: Vec::new(),
            status: HashMap::new(),
            appkey: appkey.to_string(),
        }
    }

    /// Extracts a `Vec` of system IDs (Calil library IDs) from the `libraries` map.
    fn systemids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for (library, _) in &self.libraries {
            ids.push(library.to_string());
        }
        ids
    }

    /// Extracts a `Vec` of ISBNs from the `books` currently stored in `BookDb`.
    ///
    /// These ISBNs are typically those found during the `search` operation.
    fn isbns(&self) -> Vec<String> {
        let mut isbn_list = Vec::new();
        for book in &self.books {
            isbn_list.push(book.isbn.to_string());
        }
        isbn_list
    }

    /// Retrieves the title of a book given its normalized ISBN.
    ///
    /// This method searches through the `books` found during the last `search` operation.
    ///
    /// # Arguments
    ///
    /// * `isbn` - The normalized ISBN string of the book to look up.
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` containing the book's title if found.
    /// Returns `Err(String)` if the ISBN is not found among the stored books.
    ///
    /// # Errors
    ///
    /// Returns an `Err` message if no book matching the provided ISBN is found.
    pub fn book_title(&self, isbn: &str) -> Result<String, String> {
        for book in &self.books {
            if book.isbn == isbn {
                return Ok(book.title.to_string());
            }
        }

        Err(format!("ISBN '{}' not found", isbn))
    }

    /// Searches for books based on the given title, then queries their availability status.
    ///
    /// This function first attempts to search for books using `search_book_c` (CiNii Books).
    /// If no results are found from CiNii, it falls back to `search_book_k` (NDL Search).
    /// After finding books, it then queries their availability status using the Calil API.
    /// The found books and their status are stored internally.
    ///
    /// # Arguments
    ///
    /// * `title` - The title of the book to search for.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the number of books found (`usize`) on success.
    /// Returns `GenericError` if any of the search or status query operations fail.
    ///
    /// # Errors
    ///
    /// Can return `GenericError` if:
    /// - `search_book_c` or `search_book_k` encounter network or parsing errors.
    /// - `query_book_status` encounters network, API, or JSON parsing errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use std::collections::HashMap;
    /// # use crate::BookDb;
    /// # use crate::GenericError;
    /// # async fn run() -> Result<(), GenericError> {
    /// let appkey = "YOUR_CALIL_API_KEY";
    /// let mut libraries = HashMap::new();
    /// libraries.insert("Tokyo_NDL".to_string(), "国立国会図書館".to_string());
    ///
    /// let mut book_db = BookDb::new(appkey, &libraries);
    /// let num_found = book_db.search("プログラミングRust").await?;
    /// println!("Found {} books.", num_found);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn search(&mut self, title: &str) -> Result<usize, GenericError> {
        self.books = search_book_c(title).await?;
        if self.books.len() == 0 {
            info!("Switch to search K.");
            self.books = search_book_k(title).await?;
        }
        self.title = title.to_string();
        self.status = query_book_status(&self.appkey, &self.isbns(), &self.systemids()).await?;

        Ok(self.books.len())
    }

    /// Prints the search results and library availability status to the console.
    ///
    /// This method iterates through the `status` of the books found and displays
    /// their titles, associated libraries, and availability details in a human-readable format.
    ///
    /// # Warnings
    ///
    /// Logs `warn!` messages if:
    /// - An ISBN found in `self.status` does not correspond to a book title in `self.books`.
    /// - A library ID found in `self.status` does not have a corresponding name in `self.libraries`.
    /// - A `reserveurl` is unexpectedly `None` for a `LibraryDetails` entry.
    /// - A `libkey` is unexpectedly `None` for a `LibraryDetails` entry (though this should only
    ///   happen if `status` is not "OK" or "Cache").
    pub fn display(&self) {
        println!("[{}]", self.title);
        println!("");
        for (isbn, systemids) in &self.status {
            match self.book_title(&isbn) {
                Ok(title) => println!("{}", title),
                Err(why) => {
                    warn!("{}", why);
                    continue;
                }
            }
            for (systemid, detail) in systemids {
                let libname = match self.libraries.get(systemid) {
                    Some(name) => name,
                    None => {
                        warn!("Library '{}' not found", systemid);
                        continue;
                    }
                };

                if detail.status == "Error" {
                    println!("- {}: Error", libname);
                    continue;
                }
                let url = match &detail.reserveurl {
                    Some(url) => url,
                    None => {
                        warn!("reserveurl not found.");
                        continue;
                    }
                };
                match &detail.libkey {
                    Some(lib) => {
                        for (loc, stat) in lib {
                            println!("- {}({}): {}({})", libname, loc, stat, url);
                        }
                    }
                    None => {
                        warn!("No libkey.");
                        continue;
                    }
                }
            }
        }
    }

    /// Converts the search results and library availability status into a JSON string.
    ///
    /// The output JSON structure is defined by `BookDbJson` and `BookJson`.
    ///
    /// # Returns
    ///
    /// A `String` containing the JSON representation of the search query and book statuses.
    ///
    /// # Panics
    ///
    /// This method can `panic` if `serde_json::to_string` fails. This typically only happens
    /// if there's a problem serializing the data structures, which is rare for well-formed data.
    ///
    /// # Warnings
    ///
    /// Logs `warn!` messages if:
    /// - An ISBN found in `self.status` does not correspond to a book title in `self.books`.
    /// - A library ID found in `self.status` does not have a corresponding name in `self.libraries`.
    pub fn json(&self) -> String {
        let mut book_json = BookDbJson {
            query: self.title.clone(),
            books: Vec::new(),
        };
        for (isbn, systemids) in &self.status {
            let mut book_info = match self.book_title(&isbn) {
                Ok(title) => BookJson::new(&title),
                Err(why) => {
                    warn!("{}", why);
                    continue;
                }
            };
            for (systemid, detail) in systemids {
                let libname = match self.libraries.get(systemid) {
                    Some(name) => name,
                    None => {
                        warn!("Library '{}' not found", systemid);
                        continue;
                    }
                };
                book_info.add_library(libname, &detail);
            }
            book_json.books.push(book_info);
        }
        serde_json::to_string(&book_json).unwrap()
    }
}

/// Represents a single book's information and its availability across different libraries
/// in a JSON-friendly format. This structure is used within the `BookDbJson` output.
/// 
/// ```json
/// [
///   {
///     "book_title": "退屈なことはPythonにやらせよう : ノンプログラマーにもできる自動化処理プログラミング",
///     "libraries": {
///       "国立国会図書館": {
///         "status": "OK",
///         "libkey": {
///           "東京本館": "蔵書あり",
///           "関西館": "蔵書あり"
///         },
///         "reserveurl": "https://ndlsearch.ndl.go.jp/books/R100000002-I028183776"
///       },
///       "大阪府立図書館": {
///         "status": "OK",
///         "libkey": {
///           "中央": "貸出可"
///         },
///         "reserveurl": "https://www.library.pref.osaka.jp/opw/OPW/OPWSRCHTYPE.CSP?DB=LIB&FROMFLG=1&BID=B13440282"
///       },
///       "東京都立図書館": {
///         "status": "OK",
///         "libkey": {
///           "中央": "館内のみ"
///         },
///         "reserveurl": "https://catalog.library.metro.tokyo.lg.jp/winj/opac/switch-detail-iccap.do?bibid=1153008801"
///       }
///     }
///   },
///   {
///     "book_title": "退屈なことはPythonにやらせよう : ノンプログラマーにもできる自動化処理プログラミング",
///     "libraries": {
///       "東京都立図書館": {
///         "status": "OK",
///         "libkey": {
///           "中央": "館内のみ"
///         },
///         "reserveurl": "https://catalog.library.metro.tokyo.lg.jp/winj/opac/switch-detail-iccap.do?bibid=1154154524"
///       },
///       "国立国会図書館": {
///         "status": "OK",
///         "libkey": {
///           "東京本館": "蔵書あり"
///         },
///         "reserveurl": "https://ndlsearch.ndl.go.jp/books/R100000002-I032703594"
///       },
///       "大阪府立図書館": {
///         "status": "OK",
///         "libkey": {
///           "中央": "貸出中"
///         },
///         "reserveurl": "https://www.library.pref.osaka.jp/opw/OPW/OPWSRCHTYPE.CSP?DB=LIB&FROMFLG=1&BID=B14245288"
///       }
///     }
///   }
/// ]
/// ```
#[derive(Deserialize, Serialize, Debug)]
pub struct BookJson {
    pub book_title: String,
    pub libraries: HashMap<LibraryName, LibraryDetails>,
}

impl BookJson {
    /// Creates a new `BookJson` instance with the given title and an empty set of libraries.
    ///
    /// # Arguments
    ///
    /// * `title` - The title of the book.
    ///
    /// # Returns
    ///
    /// A new `BookJson` instance.
    pub fn new(title: &str) -> BookJson {
        BookJson {
            book_title: title.to_string(),
            libraries: HashMap::new(),
        }
    }

    /// Adds library availability details for a specific library to the `BookJson` instance.
    ///
    /// # Arguments
    ///
    /// * `library_name` - The human-readable name of the library (e.g., "国立国会図書館").
    /// * `library_detail` - The `LibraryDetails` struct containing the status, libkey, and reserve URL.
    pub fn add_library(&mut self, library_name: &str, library_detail: &LibraryDetails) {
        self.libraries.insert(library_name.to_string(), library_detail.clone());
    }
}


/// Represents the overall JSON structure for book search results, including the original query
/// and a list of `BookJson` items. This is the top-level structure returned by `BookDb::json()`.
#[derive(Deserialize, Serialize, Debug)]
pub struct BookDbJson {
    /// The original search query (book title).
    pub query: String,
    /// A vector of `BookJson` instances, each representing a found book and its library statuses.
    pub books: Vec<BookJson>,
}