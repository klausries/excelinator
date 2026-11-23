/* Compiles against the header and exercises write → read. */
#include "xlcells.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#define S(s) (const uint8_t *)(s), strlen(s)

int main(void) {
    XlWriter *w = xl_writer_new();
    XlBytes *err = NULL;
    assert(xl_writer_to_bytes(w, &err) == NULL && err != NULL);
    printf("expected error: %.*s\n", (int)xl_bytes_len(err), (const char *)xl_bytes_data(err));
    xl_bytes_free(err);

    assert(xl_writer_add_sheet(w, S("Hey, Dude")) == 0);
    uint32_t date = xl_writer_add_style(w, S("D-MMM-YY"), true, false);
    assert(xl_writer_write(w, 0, 0, 0, XL_TEXT, 0, S("text"), NULL, 0, 0) == 0);
    assert(xl_writer_write(w, 0, 0, 1, XL_NUMBER, 42, NULL, 0, NULL, 0, 0) == 0);
    assert(xl_writer_write(w, 0, 1, 0, XL_NUMBER, xl_date_serial(2005, 7, 20, 12, 0, 0, 0), NULL, 0, NULL, 0, date) == 0);
    assert(xl_writer_write(w, 0, 2, 0, XL_BLANK, 0, NULL, 0, S("SUM(B1;10)"), 0) == 0);
    assert(xl_writer_write(w, 7, 0, 0, XL_NUMBER, 1, NULL, 0, NULL, 0, 0) == -1);
    XlBytes *file = xl_writer_to_bytes(w, NULL);
    assert(file);

    XlTable *t = xl_read(xl_bytes_data(file), xl_bytes_len(file), NULL);
    assert(t && xl_table_len(t) == 4 && xl_table_sheet_count(t) == 1);
    size_t n;
    const uint8_t *name = xl_table_sheet_name(t, 0, &n);
    assert(n == 9 && memcmp(name, "Hey, Dude", 9) == 0);
    assert(xl_table_kind(t)[0] == XL_TEXT && xl_table_text_offsets(t)[1] == 4 && memcmp(xl_table_text_data(t), "text", 4) == 0);
    assert(xl_table_num(t)[1] == 42.0 && xl_table_num(t)[2] == 38553.5 && xl_table_is_date(t)[2] == 1);
    const uint32_t *fo = xl_table_formula_offsets(t);
    assert(fo[4] - fo[3] == 10 && memcmp(xl_table_formula_data(t) + fo[3], "SUM(B1,10)", 10) == 0);
    assert(isnan(xl_date_serial(1800, 1, 1, 0, 0, 0, 0)));
    assert(xl_read((const uint8_t *)"nope", 4, &err) == NULL && err);
    xl_bytes_free(err);
    xl_table_free(t);
    xl_bytes_free(file);
    xl_writer_free(w);
    puts("ok");
    return 0;
}
