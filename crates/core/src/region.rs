//! Which country's prices to ask a store for.
//!
//! Every store that sells things sells them at a different price, in a different
//! money, in each country. The store is asked for the country this device is set to
//! (`en_CA` is Canada), so a price shown is the price the person would pay; where the
//! device does not say, the United States'. The setting `country` overrides it.

/// The countries offered in Settings: code, and the name each gives itself. A
/// country that is not listed can still be the system's own.
pub fn countries() -> &'static [(String, String)] {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        countries: Vec<(String, String)>,
    }
    static LIST: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let mut all = serde_json::from_str::<File>(include_str!("../data/countries.json"))
            .map(|file| file.countries)
            .unwrap_or_default();
        all.retain(|(code, name)| country(code).as_deref() == Some(code.as_str()) && !name.trim().is_empty());
        all
    })
}

/// What a country calls itself, or its code where the list does not have it.
pub fn name_of(code: &str) -> String {
    countries().iter().find(|(known, _)| known == code).map_or_else(|| code.to_string(), |(_, name)| name.clone())
}

/// The country a locale names, as two capital letters: `en_CA.UTF-8` and `fr-CA` are
/// `CA`, `zh-Hans-CN` is `CN`. Nothing for a locale that names none (`C`, `fr`).
pub fn country_of(locale: &str) -> Option<String> {
    let locale = locale.split(['.', '@']).next().unwrap_or("");
    locale.split(['-', '_']).skip(1).find_map(country)
}

/// A country code as stores want it, or nothing if this is not one.
pub fn country(code: &str) -> Option<String> {
    let code = code.trim();
    (code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())).then(|| code.to_ascii_uppercase())
}

/// The country the environment names, the way programs on Linux and macOS are told:
/// the variable for money first.
pub fn from_env() -> Option<String> {
    ["LC_ALL", "LC_MONETARY", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter(|value| !value.trim().is_empty())
        .find_map(|value| country_of(&value))
}

/// The money a country pays in, where a store has to be told (GOG): the countries of
/// the currencies GOG sells in. Anywhere else, United States dollars, as GOG itself does.
pub fn currency_of(country: &str) -> &'static str {
    const EURO: &[&str] = &[
        "AT", "BE", "CY", "DE", "EE", "ES", "FI", "FR", "GR", "HR", "IE", "IT", "LT", "LU", "LV", "MT", "NL", "PT",
        "SI", "SK",
    ];
    match country {
        code if EURO.contains(&code) => "EUR",
        "GB" => "GBP",
        "CA" => "CAD",
        "AU" => "AUD",
        "NZ" => "NZD",
        "PL" => "PLN",
        "BR" => "BRL",
        "CN" => "CNY",
        "SE" => "SEK",
        "NO" => "NOK",
        "DK" => "DKK",
        "CH" => "CHF",
        "UA" => "UAH",
        _ => "USD",
    }
}

/// An amount in hundredths, written the way that money is usually written: `$16.99`,
/// `€13.99`, `59.99 PLN`. Nothing costs "Free".
pub fn money(hundredths: u64, currency: &str) -> String {
    if hundredths == 0 {
        return "Free".to_string();
    }
    let amount = format!("{}.{:02}", hundredths / 100, hundredths % 100);
    match currency {
        "USD" | "CAD" | "AUD" | "NZD" => format!("${amount}"),
        "EUR" => format!("€{amount}"),
        "GBP" => format!("£{amount}"),
        "BRL" => format!("R${amount}"),
        "CNY" => format!("¥{amount}"),
        _ => format!("{amount} {currency}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_names_its_country_or_none() {
        assert_eq!(country_of("en_CA.UTF-8").as_deref(), Some("CA"));
        assert_eq!(country_of("fr-ca").as_deref(), Some("CA"));
        assert_eq!(country_of("zh-Hans-CN").as_deref(), Some("CN"));
        assert_eq!(country_of("sr_RS@latin").as_deref(), Some("RS"));
        for none in ["C", "POSIX", "fr", "", "en_", "en_419", "C.UTF-8"] {
            assert_eq!(country_of(none), None, "{none}");
        }
        assert_eq!(country("ca").as_deref(), Some("CA"));
        assert!(country("CAN").is_none() && country("c&").is_none() && country("").is_none());
    }

    #[test]
    fn the_countries_offered_are_each_listed_once() {
        let all = countries();
        assert!(all.len() >= 40, "{}", all.len());
        let mut codes: Vec<&str> = all.iter().map(|(code, _)| code.as_str()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
        assert_eq!((name_of("CA"), name_of("DE"), name_of("ZZ")), ("Canada".into(), "Deutschland".into(), "ZZ".into()));
    }

    #[test]
    fn money_is_written_as_people_write_it() {
        assert_eq!(money(1699, "CAD"), "$16.99");
        assert_eq!(money(1399, "EUR"), "€13.99");
        assert_eq!(money(905, "GBP"), "£9.05");
        assert_eq!(money(5999, "PLN"), "59.99 PLN");
        assert_eq!(money(0, "EUR"), "Free");
        assert_eq!(
            (currency_of("CA"), currency_of("DE"), currency_of("JP"), currency_of("US")),
            ("CAD", "EUR", "USD", "USD")
        );
    }
}
