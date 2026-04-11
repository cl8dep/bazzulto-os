#pragma once

// Character classification and conversion — C11 §7.4
// All functions operate on (unsigned char) values or EOF.

// Classification
int isalpha(int c);   // letter (a-z, A-Z)
int isdigit(int c);   // decimal digit (0-9)
int isalnum(int c);   // letter or digit
int isspace(int c);   // whitespace (' ', '\t', '\n', '\r', '\f', '\v')
int isupper(int c);   // uppercase letter (A-Z)
int islower(int c);   // lowercase letter (a-z)
int isprint(int c);   // printable character (0x20–0x7E)
int isgraph(int c);   // printable and non-space (0x21–0x7E)
int ispunct(int c);   // printable, non-space, non-alnum
int iscntrl(int c);   // control character (0x00–0x1F, 0x7F)
int isxdigit(int c);  // hexadecimal digit (0-9, a-f, A-F)
int isblank(int c);   // space or tab

// Conversion
int toupper(int c);   // convert to uppercase
int tolower(int c);   // convert to lowercase
