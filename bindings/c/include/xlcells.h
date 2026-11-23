/* xlcells: read and write Excel workbooks as cell tables. C interface.
 *
 * Strings are UTF-8 with explicit length; a null pointer means "none".
 * Objects returned by xl_writer_new, xl_writer_to_bytes and xl_read must be
 * released with their *_free function. Column pointers of a table stay valid
 * until xl_table_free. Error messages are XlBytes (UTF-8) to be freed by the caller.
 */
#ifndef XLCELLS_H
#define XLCELLS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct XlWriter XlWriter;
typedef struct XlBytes XlBytes;
typedef struct XlTable XlTable;

enum { XL_BLANK = 0, XL_NUMBER = 1, XL_TEXT = 2, XL_BOOL = 3, XL_ERROR = 4 };

uint8_t *xl_alloc(size_t len);
void xl_dealloc(uint8_t *ptr, size_t len);

XlWriter *xl_writer_new(void);
void xl_writer_free(XlWriter *w);
int32_t xl_writer_add_sheet(XlWriter *w, const uint8_t *name, size_t name_len);
uint32_t xl_writer_add_style(XlWriter *w, const uint8_t *num_format, size_t num_format_len, bool bold, bool italic);
int32_t xl_writer_write(XlWriter *w, uint32_t sheet, uint32_t row, uint32_t col, uint8_t kind, double num,
                        const uint8_t *text, size_t text_len, const uint8_t *formula, size_t formula_len, uint32_t style);
void xl_writer_merge(XlWriter *w, uint32_t sheet, uint32_t r1, uint32_t r2, uint32_t c1, uint32_t c2);
void xl_writer_set_col_width(XlWriter *w, uint32_t sheet, uint32_t col, uint32_t width);
void xl_writer_set_date1904(XlWriter *w, bool on);
XlBytes *xl_writer_to_bytes(const XlWriter *w, XlBytes **error);

const uint8_t *xl_bytes_data(const XlBytes *b);
size_t xl_bytes_len(const XlBytes *b);
void xl_bytes_free(XlBytes *b);

XlTable *xl_read(const uint8_t *data, size_t len, XlBytes **error);
void xl_table_free(XlTable *t);
size_t xl_table_len(const XlTable *t);
bool xl_table_date1904(const XlTable *t);
const uint32_t *xl_table_sheet(const XlTable *t);
const uint32_t *xl_table_row(const XlTable *t);
const uint32_t *xl_table_col(const XlTable *t);
const uint8_t *xl_table_kind(const XlTable *t);
const double *xl_table_num(const XlTable *t);
const uint8_t *xl_table_is_date(const XlTable *t);
const uint32_t *xl_table_style(const XlTable *t);
const uint32_t *xl_table_text_offsets(const XlTable *t);    /* len + 1 entries */
const uint8_t *xl_table_text_data(const XlTable *t);
const uint32_t *xl_table_formula_offsets(const XlTable *t); /* len + 1 entries */
const uint8_t *xl_table_formula_data(const XlTable *t);
size_t xl_table_sheet_count(const XlTable *t);
const uint8_t *xl_table_sheet_name(const XlTable *t, size_t i, size_t *len);
size_t xl_table_num_format_count(const XlTable *t);
const uint8_t *xl_table_num_format(const XlTable *t, size_t i, size_t *len);

double xl_date_serial(int32_t year, uint32_t month, uint32_t day, uint32_t hour, uint32_t minute, uint32_t second, uint32_t microsecond);

#ifdef __cplusplus
}
#endif
#endif
