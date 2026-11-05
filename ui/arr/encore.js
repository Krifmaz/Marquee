// Marquee's Encore look for the Sonarr and Radarr web apps: the upstream dark theme with every colour taken from
// the kit's tokens (tokens.css, loaded by index.html), so these pages match the rest of Marquee. Copied into
// frontend/src/Styles/Themes/ by ui/build.sh. Keys an app doesn't use are ignored by it.
const dark = require('./dark');

const bg = 'var(--bg)';
const bg2 = 'var(--bg-2)';
const panel = 'var(--panel)';
const panel2 = 'var(--panel-2)';
const border = 'var(--border)';
const text = 'var(--text)';
const muted = 'var(--muted)';
const faint = 'var(--faint)';
const accent = 'var(--accent)';
const accent2 = 'var(--accent-2)';
const onAccent = 'var(--on-accent)';
const ok = 'var(--ok)';
const warn = 'var(--warn)';
const bad = 'var(--err)';
const info = 'var(--info)';
const accentSoft = 'rgba(var(--accent-rgb), .12)';
const accentRing = 'rgba(var(--accent-rgb), .35)';

module.exports = {
  ...dark,

  textColor: text,
  defaultColor: text,
  disabledColor: faint,
  dimColor: faint,
  offWhite: text,
  primaryColor: accent,
  selectedColor: accent2,
  successColor: ok,
  dangerColor: bad,
  warningColor: warn,
  infoColor: info,
  queueColor: info,
  sonarrBlue: accent,
  helpTextColor: muted,
  darkGray: muted,
  gray: muted,
  lightGray: border,
  mediumGray: muted,

  themeBlue: accent,
  themeAlternateBlue: accent2,
  themeAlternateYellow: accent2,
  themeRed: bad,
  themeDarkColor: panel2,
  themeLightColor: panel2,
  pageBackground: bg,
  pageFooterBackground: bg2,
  pageFooterBackgroud: bg2,

  torrentColor: ok,
  usenetColor: info,

  inverseLabelColor: panel2,
  inverseLabelTextColor: text,
  disabledLabelColor: faint,
  infoTextColor: onAccent,

  defaultLinkHoverColor: text,
  linkColor: accent2,
  linkHoverColor: accent,

  pageHeaderBackgroundColor: bg2,

  sidebarColor: text,
  sidebarBackgroundColor: bg2,
  sidebarActiveBackgroundColor: panel,

  toolbarColor: text,
  toolbarBackgroundColor: panel,
  toolbarMenuItemBackgroundColor: panel2,
  toolbarMenuItemHoverBackgroundColor: border,
  toolbarLabelColor: muted,

  borderColor: border,
  inputBorderColor: border,
  inputBoxShadowColor: 'transparent',
  inputFocusBorderColor: accent,
  inputFocusBoxShadowColor: accentRing,
  inputErrorBorderColor: bad,
  inputErrorBoxShadowColor: 'rgba(239, 68, 68, .35)',
  inputWarningBorderColor: warn,
  inputWarningBoxShadowColor: 'rgba(245, 158, 11, .35)',
  colorImpairedGradient: border,
  colorImpairedGradientDark: panel2,
  colorImpairedDangerGradient: bad,
  colorImpairedWarningGradient: warn,
  colorImpairedPrimaryGradient: accent2,
  colorImpairedGrayGradient: faint,

  defaultButtonTextColor: text,
  defaultBackgroundColor: panel2,
  defaultBorderColor: border,
  defaultHoverBackgroundColor: border,
  defaultHoverBorderColor: faint,

  primaryBackgroundColor: accent,
  primaryBorderColor: accent,
  primaryHoverBackgroundColor: accent2,
  primaryHoverBorderColor: accent2,

  successBackgroundColor: ok,
  successBorderColor: ok,
  successHoverBackgroundColor: ok,
  successHoverBorderColor: ok,

  warningBackgroundColor: warn,
  warningBorderColor: warn,
  warningHoverBackgroundColor: warn,
  warningHoverBorderColor: warn,

  dangerBackgroundColor: bad,
  dangerBorderColor: bad,
  dangerHoverBackgroundColor: bad,
  dangerHoverBorderColor: bad,

  iconButtonDisabledColor: faint,
  iconButtonHoverColor: accent,
  iconButtonHoverLightColor: text,

  modalBackdropBackgroundColor: 'rgba(0, 0, 0, .6)',
  modalBackgroundColor: panel,
  modalCloseButtonHoverColor: muted,

  menuItemColor: text,
  menuItemHoverColor: accent,
  menuItemHoverBackgroundColor: panel2,

  toobarButtonHoverColor: accent,
  toobarButtonSelectedColor: accent2,

  scrollbarBackgroundColor: border,
  scrollbarHoverBackgroundColor: faint,

  cardBackgroundColor: panel,
  cardShadowColor: 'rgba(0, 0, 0, .35)',
  cardAlternateBackgroundColor: panel2,
  cardCenterBackgroundColor: panel,

  alertDangerBorderColor: bad,
  alertDangerBackgroundColor: 'rgba(239, 68, 68, .1)',
  alertDangerColor: text,
  alertInfoBorderColor: info,
  alertInfoBackgroundColor: 'rgba(56, 189, 248, .1)',
  alertInfoColor: text,
  alertSuccessBorderColor: ok,
  alertSuccessBackgroundColor: 'rgba(34, 197, 94, .1)',
  alertSuccessColor: text,
  alertWarningBorderColor: warn,
  alertWarningBackgroundColor: 'rgba(245, 158, 11, .1)',
  alertWarningColor: text,

  sliderAccentColor: accent,

  inputBackgroundColor: bg2,
  inputReadOnlyBackgroundColor: bg,
  inputHoverBackgroundColor: accentSoft,
  inputSelectedBackgroundColor: accentSoft,
  advancedFormLabelColor: warn,
  disabledCheckInputColor: faint,
  disabledInputColor: faint,

  popoverTitleBackgroundColor: panel2,
  popoverTitleBorderColor: border,
  popoverBodyBackgroundColor: panel,
  popoverShadowColor: 'rgba(0, 0, 0, .35)',
  popoverArrowBorderColor: panel,
  popoverTitleBackgroundInverseColor: panel2,
  popoverTitleBorderInverseColor: border,
  popoverShadowInverseColor: 'rgba(0, 0, 0, .35)',
  popoverArrowBorderInverseColor: panel2,

  calendarTodayBackgroundColor: panel2,
  calendarBackgroundColor: panel,
  calendarBorderColor: border,
  calendarTextDim: text,
  calendarTextDimAlternate: text,

  tableRowHoverBackgroundColor: accentSoft,

  addSeriesBackgroundColor: panel,
  seriesBackgroundColor: panel,
  addMovieBackgroundColor: panel,
  movieBackgroundColor: panel,
  searchIconContainerBackgroundColor: panel2,
  collapseButtonBackgroundColor: panel,
  seasonBackgroundColor: panel2,
  episodesBackgroundColor: panel,

  progressBarFrontTextColor: onAccent,
  progressBarBackTextColor: text,
  progressBarBackgroundColor: panel2,
  logEventsBackgroundColor: panel
};
