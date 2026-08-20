/**
 * Stats feature barrel (R7.2).
 *
 * Public surface for downstream shells. Keep exports narrow — helpers
 * are pure so additional consumers can still reach into `statsHelpers`
 * directly when they need something unusual.
 */

export * from './statsHelpers';
export { StatsOverlay } from './StatsOverlay';
export type { StatsOverlayProps } from './StatsOverlay';
export { QQPlot } from './QQPlot';
export type { QQPlotProps } from './QQPlot';
export { MonteCarloStatsPanel } from './MonteCarloStatsPanel';
export type {
  MonteCarloStatsPanelProps,
  MonteCarloStatsOutcome,
} from './MonteCarloStatsPanel';
export { SweepStatsPanel } from './SweepStatsPanel';
export type { SweepStatsPanelProps, SweepStatsMetric } from './SweepStatsPanel';
