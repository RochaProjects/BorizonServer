//! Estado e transições vanilla de clima por dimensão.
//!
//! `settle_levels`, usado pelo recurso custom Horizon Still para congelar a
//! transição visual, fica fora deste módulo server-only.

use horizon_core::random::{LegacyRandom, RandomSource};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Weather {
    pub clear_time: i32,
    pub rain_time: i32,
    pub thunder_time: i32,
    pub raining: bool,
    pub thundering: bool,
    /// Nível anterior e atual da chuva, mantidos pelo ciclo vanilla.
    pub rain_level: (f32, f32),
    /// Nível anterior e atual do trovão, mantidos pelo ciclo vanilla.
    pub thunder_level: (f32, f32),
}

impl Weather {
    /// Timers de `advanceWeatherCycle`, executados quando a regra está ativa.
    pub fn tick_timers(&mut self, random: &mut LegacyRandom) {
        let mut uniform =
            |min: i32, max: i32| min + random.next_int_bound(max - min + 1).unwrap_or(0);
        if self.clear_time > 0 {
            self.clear_time -= 1;
            self.thunder_time = if self.thundering { 0 } else { 1 };
            self.rain_time = if self.raining { 0 } else { 1 };
            self.thundering = false;
            self.raining = false;
        } else {
            if self.thunder_time > 0 {
                self.thunder_time -= 1;
                if self.thunder_time == 0 {
                    self.thundering = !self.thundering;
                }
            } else if self.thundering {
                self.thunder_time = uniform(3600, 15600);
            } else {
                self.thunder_time = uniform(12000, 180000);
            }
            if self.rain_time > 0 {
                self.rain_time -= 1;
                if self.rain_time == 0 {
                    self.raining = !self.raining;
                }
            } else if self.raining {
                self.rain_time = uniform(12000, 24000);
            } else {
                self.rain_time = uniform(12000, 180000);
            }
        }
    }

    /// Avança um passo dos níveis interpolados de chuva e trovão.
    pub fn tick_levels(&mut self) {
        self.thunder_level.0 = self.thunder_level.1;
        self.thunder_level.1 =
            (self.thunder_level.1 + if self.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        self.rain_level.0 = self.rain_level.1;
        self.rain_level.1 =
            (self.rain_level.1 + if self.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    }

    /// Chuva e trovão interpolados dentro do tick; trovão é escalado pela chuva.
    pub fn levels(&self, partial: f32) -> (f32, f32) {
        let rain = self.rain_level.0 + (self.rain_level.1 - self.rain_level.0) * partial;
        let thunder =
            self.thunder_level.0 + (self.thunder_level.1 - self.thunder_level.0) * partial;
        (rain, thunder * rain)
    }
}

#[cfg(test)]
mod tests {
    use super::Weather;
    use horizon_core::random::LegacyRandom;

    #[test]
    fn clear_timer_resets_weather_then_countdown_toggles_both_states() {
        let mut weather = Weather {
            clear_time: 1,
            raining: true,
            thundering: true,
            ..Weather::default()
        };
        let mut random = LegacyRandom::new(0);

        weather.tick_timers(&mut random);
        assert_eq!(weather.clear_time, 0);
        assert!(!weather.raining);
        assert!(!weather.thundering);
        assert_eq!((weather.rain_time, weather.thunder_time), (0, 0));

        weather.rain_time = 1;
        weather.thunder_time = 1;
        weather.tick_timers(&mut random);
        assert!(weather.raining);
        assert!(weather.thundering);
        assert_eq!((weather.rain_time, weather.thunder_time), (0, 0));
    }

    #[test]
    fn active_weather_uses_vanilla_timer_ranges() {
        let mut weather = Weather {
            raining: true,
            thundering: true,
            ..Weather::default()
        };
        let mut random = LegacyRandom::new(26_300);

        weather.tick_timers(&mut random);

        assert!((3600..=15600).contains(&weather.thunder_time));
        assert!((12000..=24000).contains(&weather.rain_time));
        assert!(weather.raining);
        assert!(weather.thundering);
    }

    #[test]
    fn levels_follow_targets_and_interpolate_thunder_scaled_by_rain() {
        let mut weather = Weather {
            raining: true,
            rain_level: (0.2, 0.3),
            thunder_level: (0.5, 0.4),
            ..Weather::default()
        };

        weather.tick_levels();
        assert_eq!(weather.rain_level.0, 0.3);
        assert!((weather.rain_level.1 - 0.31).abs() < 1e-6);
        assert_eq!(weather.thunder_level.0, 0.4);
        assert!((weather.thunder_level.1 - 0.39).abs() < 1e-6);
        assert_eq!(weather.levels(0.5), (0.305, 0.305 * 0.395));
    }
}
