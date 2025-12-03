#ifndef HTTP_READER_H
#define HTTP_READER_H

#include <curl/curl.h>
#include <stddef.h>
#include <stdint.h>

#define HTTP_TIMEOUT 600L
#define HTTP_MAX_RETRIES 3

typedef struct {
  uint8_t *data;
  size_t size;
  size_t capacity;
} http_response_t;

typedef struct {
  char *url;
  CURL *curl;
  uint64_t content_length;
  uint64_t current_pos;
  int supports_ranges;
  char *user_agent;
} http_reader_t;

size_t http_write_callback(void *contents, size_t size, size_t nmemb,
                           http_response_t *response);

int http_reader_init(http_reader_t *reader, const char *url, int silent);
void http_reader_cleanup(http_reader_t *reader);
int http_reader_seek(http_reader_t *reader, uint64_t offset);
int http_reader_read_at(http_reader_t *reader, uint64_t offset, uint8_t *buffer,
                        size_t size, size_t *bytes_read);
int http_reader_read(http_reader_t *reader, uint8_t *buffer, size_t size,
                     size_t *bytes_read);
uint64_t http_reader_get_size(http_reader_t *reader);
void http_reader_set_user_agent(http_reader_t *reader, const char *user_agent);

char *format_size(uint64_t bytes);

#endif
