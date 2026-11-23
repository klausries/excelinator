#!/usr/bin/env python3
"""Golden workbook for the formula lift: formulas filled down columns, computed by LibreOffice.

Writes the formulas with openpyxl, lets LibreOffice calculate them (tools/lo_oracle.py)
and stores the result as crates/xl-polars/tests/data/lift-golden.xlsx. The test lifts
every column formula of that file and compares with the values LibreOffice stored.

In a formula `#` is the formula's own row, `~` the row above, `@` its own column.

usage: make_lift_golden.py
"""
import os, shutil, subprocess, sys, tempfile
import openpyxl
from openpyxl.utils import get_column_letter
from openpyxl.workbook.defined_name import DefinedName
from openpyxl.worksheet.formula import ArrayFormula

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "crates", "xl-polars", "tests", "data", "lift-golden.xlsx")
ERR = "=1/0"

SHEETS = {
    # numbers, text, text that reads as a number, an empty cell
    "Text": dict(
        a=[2, "x", None, "7", -1.5, "Abc", 0, 12.25],
        b=[1, 1, 1, 1, 1, "abc", "", 3],
        formulas=[
            'A#+B#', 'A#*2', '-A#', 'SUM(A#:B#)', 'COUNT(A#:B#)', 'AVERAGE(A#:B#,5)', 'MAX(A#:B#)', 'MIN(A#,B#,100)',
            'A#&"-"&B#', 'CONCATENATE(A#,B#,"|",1.5)', 'A#&""', 'B#&A#&B#',
            'A#="X"', 'A#=""', 'A#>B#', 'A#=0', 'A#<>B#', 'A#<="abc"', 'A#>=B#', 'A#=B#', '"b">A#', '5<A#',
            'IF(A#="","leer",A#)', 'IF(ISNUMBER(A#),A#*2,"")', 'IF(ISTEXT(A#),LEN(A#),0)', 'IF(ISBLANK(A#),1,0)',
            'IF(A#="x",1,IF(A#="",2,IF(ISNUMBER(A#),A#,"t")))', 'ISNONTEXT(A#)', 'ISNUMBER(B#)', 'ISTEXT(A#&"")',
            'UPPER(LEFT(A#&"abc",2))', 'MID("abcdef",LEN(A#)+1,3)&RIGHT("xyz")', 'LOWER(A#)&UPPER(B#)', 'LEN(A#)',
            'FIND("b",A#&"abc")', 'FIND("B",A#&"abc")', 'SEARCH("B",A#&"abc")', 'FIND("c",A#&"abcabc",4)', 'FIND("x",A#)',
            'FIND("-",A#&"-ü-",3)', 'LEN(A#)-FIND("b",A#&"b")', 'IF(ISNUMBER(FIND("b",A#&"abc")),"ja","nein")', 'FIND("a",A#&"a",0)', 'FIND("a",A#&"a",9)',
            'RIGHT(A#&"abc",LEN(B#))', 'LEFT(A#)', 'EXACT(A#,B#)', 'EXACT(LOWER(A#),B#)', 'N(A#)+1', 'LEFT("abc",0)&"|"',
            'IF(K!$B$1="JA",K!$A$1,0)+LEN(A#)', 'IF(K!$E$1="",1,0)+K!$E$1+LEN(B#)', 'K!$B$1&A#', 'A#*Satz+SUM(Bereich)',
            'IF(LEN(A#)>1,"lang","kurz")&IF(B#=1,"!","")',
        ],
    ),
    # error values as data
    "Fehler": dict(
        a=[6, ERR, None, 3, 1, -2, 0, 9],
        b=[2, 1, 0, 0, 4, ERR, 5, 3],
        formulas=[
            'A#/B#', 'IF(ISERROR(A#/B#),-1,A#/B#)', 'IFERROR(A#/B#,"-")', 'IFERROR(A#/B#,0)+1', 'IF(A#>1,1,0)',
            'SUM(A#:B#)', 'COUNT(A#:B#)', 'AVERAGE(A#:B#)', 'MAX(A#:B#)', 'AND(B#>0,A#>0)', 'OR(B#>0,A#>0)', 'NOT(A#>0)',
            'ISERROR(A#)', 'ISERR(B#)', 'ISNUMBER(A#)', 'IF(ISERROR(B#),"Fehler",B#*2)', 'A#+B#', 'A#&B#',
            'CHOOSE(A#+1,"null","eins","zwei",B#)', 'SQRT(A#)', 'LN(A#)', 'LOG10(B#)', 'MOD(A#,B#)', 'A#^B#',
            'IF(B#=0,NA(),A#/B#)', 'ISERROR(IF(B#=0,NA(),A#/B#))', 'IF(ISERROR(A#),0,A#)+IF(ISERROR(B#),0,B#)',
            'SUM(A$2:A#)', 'SUM(B$2:B#)', 'IFERROR(SUM(A$2:B#),-1)', 'POWER(A#,2)', 'A#%',
            'ISNA(A#/B#)', 'ISERR(A#/B#)', 'ISNA(IFERROR(A#/B#,NA()))', 'ISERR(IFERROR(A#/B#,NA()))',
            'ISNA(K!$G$1)', 'ISERR(K!$G$1)', 'ISNA(K!$F$1)',
            'A#+K!$F$1', 'IFERROR(A#*K!$F$1,B#)', 'SUM(K!$A$1:$F$1)+A#', 'COUNT(K!$A$1:$F$1,A#)', 'IF(ISERROR(MAX(K!$C$1:$F$1)),1,2)+B#',
        ],
    ),
    # running totals, growing and moving ranges, rounding
    "Lauf": dict(
        a=[1, 2, None, 4, -0.5, 2.5, 1e6, 0.125],
        b=[2, 0.5, 3, 1, 2, -1, 0.1, 8],
        formulas=[
            '@~+A#', '@~+A#-B#', 'A#+@~', '@~+1', '@~', '+@~+A#*B#', '@~-B#', 'SUM(A$2:A#)', 'AVERAGE(A$2:A#)',
            'COUNT(A$2:B#)', 'SUM(A$2:B#)-COUNT(B$2:B#)', 'SUM(A~:B#)', 'SUM(A~:A#)/2', 'A#-A~', 'B#+B~+A#',
            'SUM(A$2:A#)/SUM(A$2:A$9)', 'MAX(A~:B#)', 'A#/SUM(A$2:A$9)', 'SUM($A$2:$B$9)-A#',
            'STDEV(A#:B#)', 'STDEVP(A#:B#)', 'VAR(A#:B#,5)', 'VARP(A~:B#)', 'STDEV(A$2:A$9)+A#', 'STDEV(A#,B#,3)*SQRT(2)', 'STDEV(A#)', 'STDEV(A~:A#)', 'VAR(B$2:B$9,A#)',
            'ROUND(A#*B#,1)', 'ROUNDUP(A#/3,2)', 'ROUNDDOWN(A#/3,2)', 'TRUNC(A#*B#)', 'TRUNC(-A#/3,1)', 'INT(-A#)', 'INT(A#/B#)',
            'ROUND(-A#/8,2)', 'ROUND(A#*1.005,2)', 'MOD(A#,B#)', 'MOD(-A#,3)', 'SIGN(A#-B#)', 'ABS(A#-B#)', 'EXP(B#/4)',
            'SQRT(ABS(B#))', 'LN(A#+2)', '(A#+B#)*(A#-B#)', 'A#-B#-A#+B#', '(0.1+0.2)-0.3+A#*0', 'A#*10%', '-A#^2', 'PI()*A#',
            'IF(AND(A#>0,B#>0),A#,IF(OR(A#<0,B#<0),-1,0))', 'IF(A#>B#,"A",IF(A#=B#,"=","B"))', 'CHOOSE(2,A#,B#,"x")',
        ],
    ),
    # lookups in fixed tables on sheet K: rows 3..8 numbers ascending with values and text,
    # rows 11..15 text keys ascending
    "Suche": dict(
        a=[20, 25, "b", 5, None, 40.5, "Delta", 10],
        b=[2, 3, 2, 3, 2, 3, 2, 3],
        formulas=[
            'VLOOKUP(A#,K!$A$3:$C$8,2,FALSE)', 'VLOOKUP(A#,K!$A$3:$C$8,3,0)', 'VLOOKUP(A#,K!$A$3:$C$8,2)', 'VLOOKUP(A#,K!$A$3:$C$8,3,TRUE)',
            'VLOOKUP(A#,K!$A$11:$B$15,2,FALSE)', 'VLOOKUP(A#,K!$A$11:$B$15,2)', 'IF(ISNA(VLOOKUP(A#,Tabelle,2,FALSE)),-1,VLOOKUP(A#,Tabelle,2,FALSE))',
            'IFERROR(VLOOKUP(A#,K!$A$3:$C$8,3,FALSE),"fehlt")', 'VLOOKUP(A#*2,K!$A$3:$C$8,2,FALSE)+B#', 'VLOOKUP(A#&"",K!$A$11:$B$15,2,FALSE)',
            'VLOOKUP(A#,K!$A$3:$C$8,B#,FALSE)', 'VLOOKUP(A#,K!$A$3:$C$8,IF(B#>2,3,2),FALSE)*1', 'IFERROR(VLOOKUP(A#,K!$A$3:$C$8,B#+1,FALSE),"x")', 'VLOOKUP(A#,K!$A$3:$C$8,B#,TRUE)',
            'VLOOKUP(A#,K!$A$3:$C$8,HLOOKUP(B#,K!$H$1:$K$2,1,FALSE),FALSE)', 'VLOOKUP(A#,K!$A$3:$C$8,Satz-8,FALSE)',
            'HLOOKUP(B#,K!$H$1:$K$2,2,FALSE)', 'HLOOKUP(B#+0.5,K!$H$1:$K$2,2)', 'ISERROR(VLOOKUP(A#,K!$A$3:$C$8,2,FALSE))', 'ISNA(IF(B#=3,NA(),1))', 'ISERR(IF(B#=3,NA(),1))', 'IF(ISNA(IF(B#=3,NA(),1)),"na",2)', 'ISNA(VLOOKUP(A#,K!$A$3:$C$8,2,FALSE))', 'ISERR(VLOOKUP(A#,K!$A$3:$C$8,2,FALSE))',
            'VLOOKUP(A#,K!$A$3:$C$30,2,FALSE)', 'VLOOKUP(40.5,K!$A$3:$C$8,2,FALSE)*B#', 'VLOOKUP(B#,K!$H$1:$K$2,1,FALSE)',
        ],
    ),
    # OFFSET: fixed, moving with the row, picking by a computed position
    "Versatz": dict(
        a=[1, 2, None, "x", 0, 3, -1, 2.7],
        b=[2, 3, 2, 3, 2, 3, 2, 3],
        formulas=[
            'OFFSET(K!$A$3,K!$C$1,1)+B#', 'OFFSET(K!$A$3,B#,1)', 'OFFSET(K!$A$3,B#,2)', 'OFFSET(K!$A$3,A#,0)', 'OFFSET(K!$H$1,1,B#)',
            'OFFSET(K!$H$1,0,B#-2)*2', 'OFFSET(A#,1,0)', 'OFFSET(A#,-1,1)', 'OFFSET($B$2,B#,0)', 'OFFSET(A#,B#,0)', 'OFFSET(B#,-B#,0)',
            'SUM(OFFSET(K!$A$3,0,0,3,2))+B#', 'SUM(OFFSET(A#,0,0,1,2))', 'SUM(OFFSET(B#,-1,0,2,1))', 'AVERAGE(OFFSET(B$2,1,0,K!$D$1+1))',
            'OFFSET(Start,B#,K!$C$1)', 'OFFSET(K!$A$3,B#/2,1)', 'OFFSET(K!$A$3,-B#,0)', 'ISERROR(OFFSET(K!$A$3,-B#*2,0))', 'COUNT(OFFSET(K!$A$3,B#,2))',
            'IF(AND(B#>=OFFSET(K!$C$1,0,K!$C$1),B#<=OFFSET(K!$H$1,0,2)),OFFSET(Start,K!$D$1,1)*B#,0)', 'IF(ISTEXT(OFFSET(K!$A$3,B#,2)),"t",OFFSET(K!$A$3,B#,2)+1)',
            'OFFSET(K!$A$3,B#,)', 'SUM(OFFSET(K!$A$3,B#,1),1)', 'OFFSET(K!$A$3,2,B#-3)&"|"', 'OFFSET(Start,0,0)+OFFSET(Start,1,0)*B#+OFFSET(Start,2,Satz-10)*B#^2',
        ],
    ),
    # positions, conditions, products, date arithmetic and dates as text
    "Neu": dict(
        a=[36526, 36585.5, 36951, 37315, 40000, 45657.75, 36891, 44620],
        b=[1, 2, 3, -1, 0, 13, -2, 5],
        formulas=[
            'MATCH(B#*5,K!$A$3:$A$8,0)', 'MATCH(B#*5,K!$A$3:$A$8)', 'MATCH(B#*5,K!$A$3:$A$8,1)', 'IFERROR(MATCH(B#,K!$H$1:$K$1,0),-1)', 'MATCH("delta",K!$A$11:$A$15,0)+B#',
            'MATCH(-B#*30,K!$N$1:$N$5,-1)', 'INDEX(K!$A$3:$C$8,ABS(B#)+1,2)', 'INDEX(K!$A$3:$C$8,2,ABS(B#)+1)', 'INDEX(K!$A$3:$A$8,ABS(B#)+1)', 'INDEX(K!$H$1:$K$1,ABS(B#)+1)',
            'IFERROR(INDEX(K!$A$3:$C$8,B#+3,1),"ref")', 'INDEX(Tabelle,MATCH(B#*5,K!$A$3:$A$8,0),3)', 'VLOOKUP(B#*5,K!$A$3:$C$8,MATCH("x",K!$P$1:$R$1,0),FALSE)',
            'LOOKUP(B#*7,K!$A$3:$A$8,K!$C$3:$C$8)', 'LOOKUP(B#*7,K!$A$3:$C$8)', 'ROW()', 'ROW(A#)*2', 'COLUMN()', 'COLUMN(K!$C$1)', 'ROWS(K!$A$3:$C$8)+B#',
            'SUMPRODUCT(K!$A$3:$A$5,K!$B$3:$B$5)+B#', 'SUMPRODUCT(A#:B#,K!$C$1:$D$1)', 'SUMPRODUCT(A~:A#,B~:B#)',
            'SUMIF(K!$A$3:$A$8,B#*5,K!$B$3:$B$8)', 'SUMIF(K!$A$3:$A$8,">"&B#*5,K!$B$3:$B$8)', 'COUNTIF(K!$A$3:$A$8,">=20")+B#', 'COUNTIF(K!$A$11:$A$15,"*e*")', 'COUNTIF(K!$A$11:$A$15,"<>b")',
            'SUMIF(K!$A$3:$A$8,B#*5)', 'AVERAGEIF(K!$A$3:$A$8,"<"&ABS(B#)*10,K!$B$3:$B$8)', 'SUMIFS(K!$B$3:$B$8,K!$A$3:$A$8,">5",K!$A$3:$A$8,"<="&ABS(B#)*20)',
            'COUNTIFS(K!$A$3:$A$8,">0",K!$C$3:$C$8,"z*")', 'COUNTIF(B$2:B#,">0")', 'SUMIF(B$2:B#,"<3",A$2:A#)', 'AVERAGEIF(B$2:B#,">=0",A$2:A#)', 'SUMIF(B$2:B$9,B#)', 'COUNTIF(K!$A$11:$A$15,K!$A$11)+B#',
            'RANK(B#,B$2:B$9)', 'RANK(B#,B$2:B$9,1)', 'COUNTA(K!$A$3:$C$8)+B#', 'COUNTA(A#:B#,1)',
            'EDATE(A#,B#)', 'EOMONTH(A#,B#)', 'EDATE(A#,1)-A#', 'EOMONTH(A#,0)+1', 'WORKDAY(A#,B#)', 'WORKDAY(A#,B#*3,K!$M$1:$M$4)', 'NETWORKDAYS(A#,A#+B#*9)',
            'NETWORKDAYS(A#,A#+30,K!$M$1:$M$4)', 'YEARFRAC(A#,A#+B#*40)', 'YEARFRAC(A#,EOMONTH(A#,3))', 'YEARFRAC(A#,A#+400,2)', 'YEARFRAC(A#,A#+400,3)', 'YEARFRAC(A#,EDATE(A#,B#),4)',
            'TEXT(A#,"mm/dd/yyyy")', 'TEXT(A#,"ddd")', 'TEXT(A#,"DDDD, MMMM d, yy")', 'TEXT(A#,"mmm-yy")', 'DATEVALUE(TEXT(A#,"mm/dd/yy"))', 'DATEVALUE(TEXT(A#,"yyyy-mm-dd"))',
            'DATEVALUE(TEXT(A#,"d-mmm-yyyy"))', 'ISNA(VLOOKUP(B#,K!$V$1:$W$4,2,FALSE))', 'ISERR(VLOOKUP(B#,K!$V$1:$W$4,2,FALSE))', 'IF(ISNA(VLOOKUP(B#,K!$V$1:$W$4,2,FALSE)),"na",IFERROR(VLOOKUP(B#,K!$V$1:$W$4,2,FALSE),"err"))', 'ISNA(MATCH(B#,K!$V$1:$V$4,0))', 'ISNA(ABS(VLOOKUP(B#,K!$V$1:$W$4,2,FALSE)))', 'ISNA(-VLOOKUP(B#,K!$V$1:$W$4,2,FALSE))', 'IFERROR(VLOOKUP(B#,K!$V~:$W$4,2,FALSE),-1)', 'VLOOKUP(B#*10+5,K!$A#:$C$8,2)', 'IFERROR(VLOOKUP(B#*10,K!$A$3:$C#,3),"-")', 'IFERROR(DATEVALUE("2/30/2001"),-1)+B#',
        ],
    ),
    # columns that read their own earlier rows: computed by iteration
    "Rekursion": dict(
        a=[1, 2, None, 4, -0.5, 2.5, 10, 0.125],
        b=[2, 0.5, 3, 1, 2, -1, 0.1, 8],
        formulas=[
            'IF(@~="",A#,@~*B#+A#)', 'MAX(@~,A#)', 'IF(B#>1,@~+1,0)', 'IF(ISNUMBER(@~),@~+B#,B#)', 'IF(@~="","N/A",IF(LEN(@~)>3,"x",@~&"a"))',
            'IF(DAY(36526+ROW())=1,B#,N(@~)+B#)', '@~*0.9+B#', 'IF(@~>3,@~-3,@~+B#)', 'IFERROR(1/(@~-1),5)', 'IF(A#="",@~,A#)', 'ROUND(@~*1.07+B#,2)',
        ],
    ),
    # finance, arrays (a formula starting with { is entered as an array formula), chains
    "Mehr": dict(
        a=[100, 250.5, None, 80, -40, 1000, 3, 12.5],
        b=[0.05, 0.1, 0, 0.07, 0.03, 0.2, 0.01, 0.12],
        formulas=[
            'PMT(B#,12,A#)', 'PMT(B#/12,24,-A#,10,1)', 'PV(B#,10,-A#)', 'PV(B#,5,-10,A#,1)', 'FV(B#,8,-A#)', 'FV(B#,3,-5,-A#,1)', 'SLN(A#,10,B#*100)',
            'NPV(B#,A#,50,K!$A$3:$B$4)', 'NPV(0.1,A#:B#)', 'XNPV(B#+0.01,K!$S$1:$S$4,K!$T$1:$T$4)', 'XIRR(K!$S$1:$S$4,K!$T$1:$T$4)+B#',
            'XNPV(0.08,A#:B#,K!$T$1:$T$2)', 'CORREL(K!$A$3:$A$8,K!$B$3:$B$8)+B#', 'CORREL(A~:A#,B~:B#)', 'MAXA(A#,B#,"x")', 'MINA(K!$A$11:$B$12,B#)',
            'TEXT(A#,"0")', 'TEXT(A#*B#,"######0.00")', 'TEXT(ABS(A#)*1000+12345,"###-##-####")', 'TEXT(B#,"0.00%")', 'TEXT(A#*1234.5,"#,##0.00")', 'TEXT(A#,"####")',
            'TEXT(36526+B#*3.7,"M/D/YYYY HH:MM")', 'DOLLAR(ABS(A#)*1234.567)', 'DOLLAR(ABS(A#)*B#,1)', 'SUMSQ(A#:B#,2)', 'AVERAGEA(K!$A$11:$B$12,A#:B#)', 'TRIM("  a  b ")&A#',
            'CHAR(65+B#*100)', 'ADDRESS(B#*100+1,3)', 'ADDRESS(2,B#*100+1,4,,"K")', 'ADDRESS(5,28,2)&B#', 'CELL("col",K!$D$7)+B#', 'CELL("row",A#)', 'CELL("address",B$3)&B#',
            'NORMDIST(B#*10,0.5,2,FALSE)', 'NORMDIST(B#*10,0.5,2,TRUE)', 'NORMSDIST(A#/100)', 'NORMSDIST(-B#*30)', 'IPMT(B#,2,12,A#)', 'PPMT(B#,3,12,-A#,100,1)',
            'GROWTH(K!$B$3:$B$5,K!$A$3:$A$5,B#*100)', 'VDB(1000,100,10,0,B#*50)', 'VDB(2400,300,10,1.5,3,1.5)', 'VDB(2400,300,10,0,B#*40,2,TRUE)', 'DDB(1000,100,5,ROUND(B#*40,0)+1)', 'COUNTIF(K!$A$1:$Z$100,">5")+B#', 'COUNTIF(K!$A$1:$Z$100,"<>5")', 'COUNTIF(K!$A$1:$Z$100,"b")', 'SUMIF(K!$A$1:$Z$100,"<=20",K!$B$1:$AA$100)', 'COUNTIF(K!$A$3:$C$8,"<>5")', 'COUNTIF(K!$A$1:$Z$100,K!$A$11)', 'PERCENTILE(K!$A$3:$A$8,B#*4)',
            'SUBTOTAL(9,K!$A$3:$B$5)+B#', 'SUBTOTAL(1,A#:B#)', 'SUBTOTAL(4,A$2:A#)', 'PRODUCT(A#,B#,2)', 'PRODUCT(K!$A$3:$A$5)*B#', 'IRR(K!$S$1:$S$4)+B#',
            'MAX(A$2:A#)', 'MIN(A$2:B#)', 'IF(B#>0.06,,1)', 'ROUND(A#,)', 'SUM(K:Fehler!$A$3)+B#', 'SUM(K:Text!$A$3:$A$4)*B#',
            '{TRANSPOSE(K!$B$3:$B$5)', '{K!$A$4:$A$6*B#', '{SUM(TRANSPOSE(K!$A$3:$A$5)*K!$B$3:$D$3)',
            '{SUM(A#:B#*K!$C$1:$D$1)', '{SUM((K!$A$3:$A$8>B#*100)*K!$B$3:$B$8)', '{SUM(IF(K!$A$3:$A$8>B#*100,K!$B$3:$B$8))', '{MAX(IF(K!$A$3:$A$8<A#,K!$A$3:$A$8,0))',
            'SUMPRODUCT((K!$A$3:$A$8>B#*100)*K!$B$3:$B$8)', 'SUMPRODUCT(--(K!$A$11:$A$15>"c"),K!$B$11:$B$15)', 'SUM(A#:B#:A#)', 'SUM(K!$A$3:K!$B$3:$C$4)',
            'COUNTIF(K!$A$3:$A$8,K!$E$1)+COUNTIF(K!$E$1:$E$3,0)', '@~+SUM(@$1:@~)*B#+1', 'MAX(Bereich2)+B#',
        ],
    ),
    # databases, INDIRECT, CHOOSE of ranges, one value from a range, arrays of other sizes
    "Datenbank": dict(
        a=[1, 2, 3, 4, 5, 6, 7, 8],
        b=[0, 1, 2, 1, 0, 2, 1, 0],
        formulas=[
            'DSUM(K!$X$1:$Z$6,"Menge",K!$AB$1:$AB$2)+B#', 'DCOUNT(K!$X$1:$Z$6,B#+2,K!$AB$1:$AB$2)', 'DAVERAGE(K!$X$1:$Z$6,"Wert",K!$AC$1:$AC$2)',
            'DMAX(K!$X$1:$Z$6,2,K!$AB$1:$AC$3)', 'IFERROR(DSUM(K!$X$1:$Z$6,"Wert",K!$AE$1:$AE$2),-1)+B#', 'DSUM(K!$X$1:$Z$6,IF(B#>0,"Wert","Menge"),K!$AD$1:$AD$2)',
            'INDIRECT("K!A3")*B#', 'INDIRECT("K!A"&(3+K!$C$1))+B#', 'SUM(INDIRECT("K!A3:B4"))+B#', 'SUM(CHOOSE(K!$D$1,K!$A$3:$A$4,K!$B$3:$B$4))+B#',
            'A$2:A$9*2', 'LEN($A$12:$ZZ$12)+B#', '{SUM(IF(K!$A$3:$A$8<15,K!$B$3:$B$6))+B#', '{SUM(K!$A$3:$A$4*K!$B$3:$B$3)',
            # large arrays over fixed ranges (600 rows), worked out element by element at translation
            '{SUM(IF((Punkt="PV")*(Art="pre")*(OFFSET(Punkt,0,3,ROWS(Punkt),4)<0),Preis*OFFSET(Punkt,0,3,ROWS(Punkt),4),0))+B#',
            '{SUM(IF((Punkt="sps")*IF((Art="firm")+(Art="econ")>0,1,0),OFFSET(Punkt,0,3,ROWS(Punkt),4)))',
            '{SUM(IF(YEAR(K!$AP$1:$AP$600)=2000+B#,K!$AJ$1:$AJ$600))', '{COUNT(IF(K!$AH$1:$AH$600="4C",K!$AK$1:$AK$600))+A#',
            '{MAX(IF(K!$AI$1:$AI$600="pre",K!$AJ$1:$AJ$600*1,-1E9))', '{SUM(IF(ISNUMBER(K!$AO$1:$AO$600),K!$AO$1:$AO$600,0))',
            '{SUM((K!$AH$1:$AH$600&K!$AI$1:$AI$600="PVpre")*K!$AM$1:$AM$600)', '{IFERROR(SUM(IF(K!$AH$1:$AH$600="PV",1/K!$AN$1:$AN$600)),-1)',
            '{AVERAGE(IF(K!$AH$1:$AH$600<>"",K!$AJ$1:$AJ$600))', '{SUM(IF(K!$AH$1:$AH$600="",1,0))', '{SUM(K!$AO$1:$AO$600*1)',
            '{SUM(IF(K!$AK$1:$AN$600>B#*3,K!$AK$1:$AN$600-B#))', '{MIN(IF(K!$AH$1:$AH$600>"PV",K!$AJ$1:$AJ$600))',
            # parentheses around references, unions, a fixed condition between a truth value and text,
            # criteria with operators in small ranges, nested SUBTOTAL, a table of whole rows
            'MAX((K!$A$3:$A$8))+B#', 'SUM((K!$A$3:$A$4,K!$B$3:$B$4))*B#', 'COUNT(((K!$A$3:$C$8)))', 'IF(K!$A$1=0,"N/A")', 'IF(K!$A$1=10,"N/A")&B#',
            'SUMIF(K!$X$2:$X$6,K!$AB$2,K!$Y$2:$Y$6)', 'SUMIF(K!$Y$2:$Y$6,K!$AB$2)+B#', 'COUNTIF(K!$Z$2:$Z$6,K!$AD$2)', 'SUBTOTAL(9,K!$AR$1:$AR$6)+B#', 'SUBTOTAL(9,K!$AS$1:$AS$6)+B#',
            'VLOOKUP(20,K!$A$3:$XFD$300,2,0)+B#', 'HLOOKUP(2,K!$H$1:$XFD$300,2,0)', 'COUNTA(K!$A$1:$AZ$65536)+B#', 'COUNTA(K!$A$3:$C$8,K!X:X)',
            '{LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AM$600)', '{LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AJ$600,FALSE)', '{INDEX(LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AM$600,TRUE,TRUE),2,3)',
            '{INDEX(LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AM$600,TRUE(),TRUE()),3,1)', '{INDEX(LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AM$600,1,1),4,2)', '{INDEX(LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AM$600,1,1),5,1)',
            '{INDEX(LINEST(K!$AN$1:$AN$600,K!$AJ$1:$AM$600,1,1),2,5)', '{INDEX(LINEST(K!$AN$1:$AN$600,K!$AK$1:$AL$600,0,1),4,1)', '{INDEX(LINEST(K!$AN$1:$AN$600),1,2)',
        ],
    ),
    # dates from 1 March 1900 on: before that LibreOffice counts differently from Excel,
    # those days are in the hand-written test
    "Datum": dict(
        a=[36526, 400, 45657.75, 40000, 73050, 36584.999, 44620, 2958465],
        b=[1, 2, 3, 12, 13, 0, -1, 31],
        formulas=[
            'YEAR(A#)', 'MONTH(A#)', 'DAY(A#)', 'WEEKDAY(A#)', 'WEEKDAY(A#,2)', 'WEEKDAY(A#,3)',
            'DATE(YEAR(A#),MONTH(A#)+1,1)', 'DATE(YEAR(A#),B#,1)', 'DATE(2000,14,0)+B#', 'DATE(YEAR(A#),MONTH(A#),B#)',
            'DATE(YEAR(A#),MONTH(A#)+1,0)-A#', 'YEAR(A#)*100+MONTH(A#)', 'IF(WEEKDAY(A#,2)>5,"WE","")', 'DATE(2024,B#,29)',
            'A#-DATE(YEAR(A#),1,1)+1', 'MONTH(A#+B#)', 'DAY(DATE(YEAR(A#),3,0))',
        ],
    ),
}

def build(path):
    wb = openpyxl.Workbook()
    wb.remove(wb.active)
    k = wb.create_sheet("K")
    k["A1"], k["B1"], k["C1"], k["D1"], k["F1"] = 10, "ja", 1, 2, ERR
    k["G1"] = "=NA()"
    for r, row in enumerate([(5, 50, "fünf"), (10, 100, "zehn"), (20, 200, "zwanzig"), (20, 999, "doppelt"), (40.5, None, 7), (100, 1000, "hundert")], start=3):
        for c, v in enumerate(row, start=1):
            if v is not None:
                k.cell(row=r, column=c, value=v)
    for r, row in enumerate([("alpha", 1), ("B", 2), ("delta", 4), ("Epsilon", 5), ("zeta", "letzter")], start=11):
        for c, v in enumerate(row, start=1):
            k.cell(row=r, column=c, value=v)
    for c, (key, v) in enumerate([(1, "eins"), (2, 22), (3, "drei"), (4, 44)], start=8):
        k.cell(row=1, column=c, value=key)
        k.cell(row=2, column=c, value=v)
    for r, v in enumerate([36528, 36531, 37316, 40001], start=1):
        k.cell(row=r, column=13, value=v)
    for r, v in enumerate([100, 50, 20, 5, -10], start=1):
        k.cell(row=r, column=14, value=v)
    for c, v in enumerate(["u", "x", "y"], start=16):
        k.cell(row=1, column=c, value=v)
    for r, (v, d) in enumerate([(-1000, 36526), (300, 36700), (400, 36900), (500.5, 37300)], start=1):
        k.cell(row=r, column=19, value=v)
        k.cell(row=r, column=20, value=d)
    wb.defined_names["Bereich2"] = DefinedName("Bereich2", attr_text="K!$A$3:OFFSET(K!$A$3,3,1)")
    for r, (key, v) in enumerate([(1, 10), (2, "=1/0"), (3, "=NA()"), (4, 40)], start=1):
        k.cell(row=r, column=22, value=key)
        k.cell(row=r, column=23, value=v)
    for r, row in enumerate([("Name", "Wert", "Menge"), ("a", 10, 1), ("b", 20, 2), ("ab", 30, 3), ("c", 40, None), ("b", 50, 5)], start=1):
        for c, v in enumerate(row, start=24):
            if v is not None:
                k.cell(row=r, column=c, value=v)
    for r, row in enumerate([("Wert", "Name", "Menge", "Unbekannt"), (">15", "=b", ">=2", ">1"), (None, "=c", None, None)], start=1):
        for c, v in enumerate(row, start=28):
            if v is not None:
                cell = k.cell(row=r, column=c, value=v)
                cell.data_type = "s"  # criteria such as =b are text
    # a large table for array formulas: key, type, price, four hours, a mixed column, a date
    keys, kinds = ["PV", "SPS", "4C", None, "pv", "Artesia"], ["pre", "firm", "econ", "pre", "other"]
    for r in range(1, 601):
        key = keys[r % len(keys)]
        if key is not None:
            k.cell(row=r, column=34, value=key)
        k.cell(row=r, column=35, value=kinds[r % len(kinds)])
        k.cell(row=r, column=36, value=round(10 + (r * 7 % 23) * 0.25, 2))
        for c in range(4):
            k.cell(row=r, column=37 + c, value=((r * (c + 3)) % 17) - 8)
        k.cell(row=r, column=41, value=("x" if r % 7 == 0 else str(r % 5) if r % 11 == 0 else r % 9))
        k.cell(row=r, column=42, value=36526 + r * 3)
    # SUBTOTAL among the cells SUBTOTAL sums: alone, and inside a larger formula
    for r, (v, w) in enumerate([(1, 1), (2, 2), ("=SUBTOTAL(9,AR1:AR2)", "=SUBTOTAL(9,AS1:AS2)*1"), (4, 4), (5, 5), ("=SUBTOTAL(9,AR4:AR5)", "=1+SUBTOTAL(9,AS4:AS5)")], start=1):
        k.cell(row=r, column=44, value=v)
        k.cell(row=r, column=45, value=w)
    wb.defined_names["Punkt"] = DefinedName("Punkt", attr_text="K!$AH$1:$AH$600")
    wb.defined_names["Art"] = DefinedName("Art", attr_text="K!$AI$1:$AI$600")
    wb.defined_names["Preis"] = DefinedName("Preis", attr_text="K!$AJ$1:$AJ$600")
    wb.defined_names["Start"] = DefinedName("Start", attr_text="K!$A$3")
    wb.defined_names["Tabelle"] = DefinedName("Tabelle", attr_text="K!$A$3:$C$8")
    wb.defined_names["Satz"] = DefinedName("Satz", attr_text="K!$A$1")
    wb.defined_names["Bereich"] = DefinedName("Bereich", attr_text="K!$C$1:$D$1")
    for name, spec in SHEETS.items():
        ws = wb.create_sheet(name)
        ws["A1"], ws["B1"] = "a", "b"
        for r, (a, b) in enumerate(zip(spec["a"], spec["b"]), start=2):
            for col, v in (("A", a), ("B", b)):
                if v is not None:
                    ws[f"{col}{r}"] = v
            for c, formula in enumerate(spec["formulas"], start=3):
                letter = get_column_letter(c)
                text = "=" + formula.lstrip("{").replace("#", str(r)).replace("~", str(r - 1)).replace("@", letter)
                if formula.startswith("{"):
                    ws[f"{letter}{r}"] = ArrayFormula(f"{letter}{r}", text)
                else:
                    ws[f"{letter}{r}"] = text
        for c, formula in enumerate(spec["formulas"], start=3):
            ws.cell(row=12, column=c, value=formula)  # the formula as text, for reading the sheet
    wb.save(path)

def main():
    work = tempfile.mkdtemp()
    src, out = os.path.join(work, "in"), os.path.join(work, "out")
    os.makedirs(src)
    build(os.path.join(src, "lift-golden.xlsx"))
    subprocess.run([sys.executable, os.path.join(HERE, "lo_oracle.py"), src, out, "xlsx"], check=True)
    found = [os.path.join(d, f) for d, _, fs in os.walk(out) for f in fs if f.endswith(".xlsx")]
    assert len(found) == 1, found
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    shutil.copy(found[0], OUT)
    print("wrote", os.path.normpath(OUT), os.path.getsize(OUT), "bytes")

if __name__ == "__main__":
    main()
