#ifndef ZIP_PARSER_H
#define ZIP_PARSER_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#ifdef ENABLE_HTTP_SUPPORT
#include "http_reader.h"
#endif

#define LOCAL_FILE_HEADER_SIG 0x04034B50
#define CENTRAL_DIR_HEADER_SIG 0x02014B50
#define EOCD_SIG 0x06054B50
#define ZIP64_EOCD_SIG 0x06064B50
#define ZIP64_EOCD_LOCATOR_SIG 0x07064B50

typedef struct {
  char name[256];
  uint64_t compressed_size;
  uint64_t uncompressed_size;
  uint64_t local_header_offset;
  uint64_t data_offset;
  uint16_t compression_method;
} zip_entry_t;

typedef struct {
  enum {
    READER_FILE
#ifdef ENABLE_HTTP_SUPPORT
    ,
    READER_HTTP
#endif
  } type;
  union {
    FILE *file;
#ifdef ENABLE_HTTP_SUPPORT
    http_reader_t http;
#endif
  } data;
  uint64_t size;
} reader_t;

// Reader functions
int reader_init_file(reader_t *reader, const char *path);
#ifdef ENABLE_HTTP_SUPPORT
int reader_init_http(reader_t *reader, const char *url, const char *user_agent,
                     int silent);
#endif
void reader_cleanup(reader_t *reader);
int reader_seek(reader_t *reader, uint64_t offset);
int reader_read(reader_t *reader, uint8_t *buffer, size_t size,
                size_t *bytes_read);
int reader_read_at(reader_t *reader, uint64_t offset, uint8_t *buffer,
                   size_t size, size_t *bytes_read);
uint64_t reader_get_size(reader_t *reader);

// ZIP parsing functions
int find_eocd(reader_t *reader, uint64_t *eocd_offset, uint16_t *num_entries);
int read_zip64_eocd(reader_t *reader, uint64_t eocd_offset, uint64_t *cd_offset,
                    uint64_t *num_entries);
int get_central_directory_info(reader_t *reader, uint64_t *cd_offset,
                               uint64_t *num_entries);
int read_central_directory_entry(reader_t *reader, zip_entry_t *entry);
int find_payload_entry(reader_t *reader, zip_entry_t *payload_entry);
int get_data_offset(reader_t *reader, zip_entry_t *entry);
int verify_payload_magic(reader_t *reader, uint64_t offset);

// Utility functions
uint32_t read_u32_le(const uint8_t *data);
uint16_t read_u16_le(const uint8_t *data);
uint64_t read_u64_le(const uint8_t *data);

#endif