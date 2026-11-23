/// The colours of the app, by what they are for, as `#rrggbb`: one list for every
/// surface.
library;

// In the manner of Excel: its green for the bar at the top and what is picked, thin grey
// lines between the cells, grey heads of rows and columns.
const light = {
  'bg': '#ffffff', 'panel': '#f3f3f3', 'line': '#d4d4d4', 'grid': '#e1e1e1', 'head': '#f5f5f5', 'text': '#242424', 'soft': '#616161',
  'brand': '#107c41', 'onBrand': '#ffffff', 'accent': '#107c41', 'link': '#0f6cbd',
  'calc': '#eef6f1', 'kept': '#fff4ce', 'own': '#fde7e9', 'pick': '#d3ecdc',
  'table': '#0f6cbd', 'cells': '#8a3ab9', 'block': '#c4590f', 'warn': '#8a5a00', 'ok': '#107c41',
  'error': '#c4314b',
};

const dark = {
  'bg': '#1f1f1f', 'panel': '#292929', 'line': '#3d3d3d', 'grid': '#333333', 'head': '#2b2b2b', 'text': '#e8e8e8', 'soft': '#a8a8a8',
  'brand': '#185c37', 'onBrand': '#ffffff', 'accent': '#5ec58a', 'link': '#62abf5',
  'calc': '#1f3328', 'kept': '#3a3214', 'own': '#3d2226', 'pick': '#24452f',
  'table': '#62abf5', 'cells': '#c084fc', 'block': '#f59e5b', 'warn': '#e0b341', 'ok': '#5ec58a',
  'error': '#c4314b',
};

/// The text on a cell that brings a fill of its own but no colour of its text: the
/// fills of workbooks are made for dark text.
const onFill = '#1c2026';

/// The colours as CSS variables (`--bg` and so on): those of a [theme] (`light`, `dark`),
/// or without one dark where the surface is.
String cssVariables([String? theme]) {
  String vars(Map<String, String> colours) => ':root{${colours.entries.map((e) => '--${e.key}:${e.value}').join(';')}}';
  return switch (theme) {
    'dark' => vars(dark),
    'light' => vars(light),
    _ => '${vars(light)}@media(prefers-color-scheme:dark){${vars(dark)}}',
  };
}
