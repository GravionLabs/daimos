//! Units and conversions between them. Every unit knows its dimension and how to reach the base
//! unit of that dimension: `base = value * factor + offset` (the offset is for temperatures).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Length,
    Mass,
    Time,
    Volume,
    Data,
    Speed,
    Temperature,
    Currency,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    pub dim: Dim,
    pub factor: f64,
    pub offset: f64,
}

const fn u(dim: Dim, factor: f64) -> Unit {
    Unit { dim, factor, offset: 0.0 }
}

/// A currency in the rates of the day: `per_eur` units of it buy one euro.
pub fn currency(per_eur: f64) -> Unit {
    u(Dim::Currency, 1.0 / per_eur)
}

/// The built-in unit named `name` (lowercase, any common spelling).
pub fn lookup(name: &str) -> Option<Unit> {
    use Dim::{Data, Length, Mass, Speed, Temperature, Time, Volume};
    let name = name.trim().trim_start_matches('°');
    Some(match name {
        // Length, base: meter.
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => u(Length, 0.001),
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => u(Length, 0.01),
        "m" | "meter" | "meters" | "metre" | "metres" => u(Length, 1.0),
        "km" | "kilometer" | "kilometers" | "kilometre" | "kilometres" => u(Length, 1000.0),
        "in" | "inch" | "inches" | "\"" => u(Length, 0.0254),
        "ft" | "foot" | "feet" | "'" => u(Length, 0.3048),
        "yd" | "yard" | "yards" => u(Length, 0.9144),
        "mi" | "mile" | "miles" => u(Length, 1609.344),
        "nmi" | "nautical mile" | "nautical miles" => u(Length, 1852.0),
        // Mass, base: kilogram.
        "mg" | "milligram" | "milligrams" => u(Mass, 1e-6),
        "g" | "gram" | "grams" => u(Mass, 0.001),
        "kg" | "kilogram" | "kilograms" | "kilo" | "kilos" => u(Mass, 1.0),
        "t" | "tonne" | "tonnes" | "ton" | "tons" => u(Mass, 1000.0),
        "oz" | "ounce" | "ounces" => u(Mass, 0.028_349_523_125),
        "lb" | "lbs" | "pound" | "pounds" => u(Mass, 0.453_592_37),
        "st" | "stone" | "stones" => u(Mass, 6.350_293_18),
        // Time, base: second.
        "ms" | "millisecond" | "milliseconds" => u(Time, 0.001),
        "s" | "sec" | "secs" | "second" | "seconds" => u(Time, 1.0),
        "min" | "mins" | "minute" | "minutes" => u(Time, 60.0),
        "h" | "hr" | "hrs" | "hour" | "hours" => u(Time, 3600.0),
        "d" | "day" | "days" => u(Time, 86_400.0),
        "w" | "wk" | "week" | "weeks" => u(Time, 604_800.0),
        // Volume, base: liter (US customary measures).
        "ml" | "milliliter" | "milliliters" | "millilitre" | "millilitres" => u(Volume, 0.001),
        "cl" => u(Volume, 0.01),
        "dl" => u(Volume, 0.1),
        "l" | "liter" | "liters" | "litre" | "litres" => u(Volume, 1.0),
        "tsp" | "teaspoon" | "teaspoons" => u(Volume, 0.004_928_921_593_75),
        "tbsp" | "tablespoon" | "tablespoons" => u(Volume, 0.014_786_764_781_25),
        "floz" | "fl oz" => u(Volume, 0.029_573_529_562_5),
        "cup" | "cups" => u(Volume, 0.236_588_236_5),
        "pt" | "pint" | "pints" => u(Volume, 0.473_176_473),
        "qt" | "quart" | "quarts" => u(Volume, 0.946_352_946),
        "gal" | "gallon" | "gallons" => u(Volume, 3.785_411_784),
        // Data, base: byte. kb, mb, ... are decimal, kib, mib, ... binary.
        "bit" | "bits" => u(Data, 0.125),
        "b" | "byte" | "bytes" => u(Data, 1.0),
        "kb" | "kilobyte" | "kilobytes" => u(Data, 1e3),
        "mb" | "megabyte" | "megabytes" => u(Data, 1e6),
        "gb" | "gigabyte" | "gigabytes" => u(Data, 1e9),
        "tb" | "terabyte" | "terabytes" => u(Data, 1e12),
        "pb" | "petabyte" | "petabytes" => u(Data, 1e15),
        "kib" => u(Data, 1024.0),
        "mib" => u(Data, 1_048_576.0),
        "gib" => u(Data, 1_073_741_824.0),
        "tib" => u(Data, 1_099_511_627_776.0),
        // Speed, base: meter per second.
        "m/s" | "mps" => u(Speed, 1.0),
        "km/h" | "kmh" | "kph" | "kmph" => u(Speed, 1.0 / 3.6),
        "mph" | "mi/h" => u(Speed, 0.447_04),
        "kn" | "kt" | "knot" | "knots" => u(Speed, 1852.0 / 3600.0),
        "ft/s" | "fps" => u(Speed, 0.3048),
        // Temperature, base: kelvin.
        "c" | "celsius" | "degc" => Unit { dim: Temperature, factor: 1.0, offset: 273.15 },
        "f" | "fahrenheit" | "degf" => {
            Unit { dim: Temperature, factor: 5.0 / 9.0, offset: 273.15 - 32.0 * 5.0 / 9.0 }
        }
        "k" | "kelvin" => Unit { dim: Temperature, factor: 1.0, offset: 0.0 },
        _ => return None,
    })
}

/// Converts `value` between two units of the same dimension.
pub fn convert(value: f64, from: Unit, to: Unit) -> Option<f64> {
    (from.dim == to.dim).then(|| ((value * from.factor + from.offset) - to.offset) / to.factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(value: f64, from: &str, to: &str) -> f64 {
        convert(value, lookup(from).unwrap(), lookup(to).unwrap()).unwrap()
    }

    #[test]
    fn well_known_conversions() {
        assert!((conv(12.0, "km", "mi") - 7.456_454).abs() < 1e-5);
        assert!((conv(70.0, "f", "c") - 21.111_111).abs() < 1e-5);
        assert!((conv(100.0, "c", "f") - 212.0).abs() < 1e-9);
        assert!((conv(0.0, "c", "k") - 273.15).abs() < 1e-9);
        assert_eq!(conv(3.0, "gb", "mb"), 3000.0);
        assert_eq!(conv(1.0, "gib", "mib"), 1024.0);
        assert_eq!(conv(1.0, "h", "s"), 3600.0);
        assert!((conv(100.0, "km/h", "mph") - 62.137_119).abs() < 1e-5);
        assert!((conv(1.0, "gal", "l") - 3.785_411_784).abs() < 1e-9);
    }

    #[test]
    fn different_dimensions_do_not_convert() {
        assert!(convert(1.0, lookup("km").unwrap(), lookup("kg").unwrap()).is_none());
        assert!(convert(1.0, lookup("c").unwrap(), lookup("m").unwrap()).is_none());
    }

    #[test]
    fn spellings_and_unknown_units() {
        assert_eq!(lookup("°c"), lookup("c"));
        assert_eq!(lookup("miles"), lookup("mi"));
        assert!(lookup("furlong").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn currencies_convert_through_the_euro() {
        let usd = currency(1.25); // 1.25 USD per EUR
        let eur = currency(1.0);
        assert!((convert(100.0, usd, eur).unwrap() - 80.0).abs() < 1e-9);
        assert!((convert(80.0, eur, usd).unwrap() - 100.0).abs() < 1e-9);
        assert!(convert(1.0, usd, lookup("km").unwrap()).is_none());
    }
}
