#define _GNU_SOURCE
#define _DEFAULT_SOURCE
#include "http_reader.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#define sleep(seconds) Sleep((seconds) * 1000)
#else
#include <unistd.h>
#endif
#ifdef _WIN32
#define strdup _strdup
#endif
#ifdef _WIN32
#define PRIu64 "llu"
#else
#include <inttypes.h>
#endif

static int g_curl_initialized = 0;
static int g_size_info_shown = 0;
static int g_ranges_warning_shown = 0;

size_t http_write_callback(void *contents, size_t size, size_t nmemb,
                           http_response_t *response) {
  size_t total_size = size * nmemb;

  if (response->size + total_size > response->capacity) {
    size_t new_capacity = response->capacity * 2;
    if (new_capacity < response->size + total_size) {
      new_capacity = response->size + total_size + 8192;
    }

    uint8_t *new_data = realloc(response->data, new_capacity);
    if (!new_data) {
      return 0;
    }

    response->data = new_data;
    response->capacity = new_capacity;
  }

  memcpy(response->data + response->size, contents, total_size);
  response->size += total_size;

  return total_size;
}

char *format_size(uint64_t bytes) {
  static char buffer[32];
  const char *units[] = {"B", "KB", "MB", "GB", "TB"};
  int unit_idx = 0;
  double size = (double)bytes;

  while (size >= 1024.0 && unit_idx < 4) {
    size /= 1024.0;
    unit_idx++;
  }

  snprintf(buffer, sizeof(buffer), "%.2f %s", size, units[unit_idx]);
  return buffer;
}

void http_reader_set_user_agent(http_reader_t *reader, const char *user_agent) {
  if (reader->user_agent) {
    free(reader->user_agent);
  }
  reader->user_agent = user_agent ? strdup(user_agent) : NULL;
}

int http_reader_init(http_reader_t *reader, const char *url, int silent) {
  if (!g_curl_initialized) {
    curl_global_init(CURL_GLOBAL_DEFAULT);
    g_curl_initialized = 1;
  }

  memset(reader, 0, sizeof(http_reader_t));

  reader->url = strdup(url);
  if (!reader->url) {
    return -1;
  }

  reader->curl = curl_easy_init();
  if (!reader->curl) {
    free(reader->url);
    return -1;
  }

  curl_easy_setopt(reader->curl, CURLOPT_URL, reader->url);
  curl_easy_setopt(reader->curl, CURLOPT_TIMEOUT, HTTP_TIMEOUT);
  curl_easy_setopt(reader->curl, CURLOPT_FOLLOWLOCATION, 1L);
  curl_easy_setopt(reader->curl, CURLOPT_MAXREDIRS, 10L);
  const char *ua = reader->user_agent
                       ? reader->user_agent
                       : "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
                         "(KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
  curl_easy_setopt(reader->curl, CURLOPT_USERAGENT, ua);

  curl_easy_setopt(reader->curl, CURLOPT_NOBODY, 1L);
  curl_easy_setopt(reader->curl, CURLOPT_WRITEFUNCTION, NULL);

  int retry_count = 0;
  CURLcode res;

  while (retry_count < HTTP_MAX_RETRIES) {
    res = curl_easy_perform(reader->curl);
    if (res == CURLE_OK) {
      break;
    }
    retry_count++;
    if (retry_count < HTTP_MAX_RETRIES) {
      sleep((unsigned int)(2 * retry_count));
    }
  }

  if (res != CURLE_OK) {
    fprintf(stderr, "Failed to connect after %d retries: %s\n",
            HTTP_MAX_RETRIES, curl_easy_strerror(res));
    curl_easy_cleanup(reader->curl);
    free(reader->url);
    return -1;
  }

  curl_off_t content_length_t;
  curl_easy_getinfo(reader->curl, CURLINFO_CONTENT_LENGTH_DOWNLOAD_T,
                    &content_length_t);
  if (content_length_t < 0) {
    fprintf(stderr, "Could not determine content length\n");
    curl_easy_cleanup(reader->curl);
    free(reader->url);
    return -1;
  }
  reader->content_length = (uint64_t)content_length_t;

  long response_code;
  curl_easy_getinfo(reader->curl, CURLINFO_RESPONSE_CODE, &response_code);

  curl_easy_setopt(reader->curl, CURLOPT_NOBODY, 0L);
  curl_easy_setopt(reader->curl, CURLOPT_WRITEFUNCTION, http_write_callback);

  struct curl_slist *headers = NULL;
  headers = curl_slist_append(headers, "Range: bytes=0-1023");
  curl_easy_setopt(reader->curl, CURLOPT_HTTPHEADER, headers);

  http_response_t test_response = {0};
  curl_easy_setopt(reader->curl, CURLOPT_WRITEDATA, &test_response);

  res = curl_easy_perform(reader->curl);
  curl_easy_getinfo(reader->curl, CURLINFO_RESPONSE_CODE, &response_code);

  reader->supports_ranges = (res == CURLE_OK && response_code == 206);

  if (test_response.data) {
    free(test_response.data);
  }
  curl_slist_free_all(headers);

  if (!reader->supports_ranges && !g_ranges_warning_shown) {
    fprintf(stderr, "- Warning: Server doesn't support range requests. The "
                    "process may fail.\n");
    g_ranges_warning_shown = 1;
  }

  if (!silent && !g_size_info_shown) {
    fprintf(stderr, "- File size: %s\n", format_size(reader->content_length));
    g_size_info_shown = 1;
  }

  reader->current_pos = 0;
  return 0;
}

void http_reader_cleanup(http_reader_t *reader) {
  if (reader->curl) {
    curl_easy_cleanup(reader->curl);
    reader->curl = NULL;
  }
  if (reader->url) {
    free(reader->url);
    reader->url = NULL;
  }
  if (reader->user_agent) {
    free(reader->user_agent);
    reader->user_agent = NULL;
  }
}

int http_reader_seek(http_reader_t *reader, uint64_t offset) {
  if (offset > reader->content_length) {
    return -1;
  }
  reader->current_pos = offset;
  return 0;
}

int http_reader_read_at(http_reader_t *reader, uint64_t offset, uint8_t *buffer,
                        size_t size, size_t *bytes_read) {
  if (offset >= reader->content_length) {
    *bytes_read = 0;
    return 0;
  }

  uint64_t remaining = reader->content_length - offset;
  size_t to_read = (size < remaining) ? size : (size_t)remaining;

  if (to_read == 0) {
    *bytes_read = 0;
    return 0;
  }

  char range_header[256];
  snprintf(range_header, sizeof(range_header),
           "Range: bytes=%" PRIu64 "-%" PRIu64, offset, offset + to_read - 1);

  struct curl_slist *headers = NULL;
  headers = curl_slist_append(headers, range_header);
  curl_easy_setopt(reader->curl, CURLOPT_HTTPHEADER, headers);

  http_response_t response = {0};
  response.capacity = to_read + 1024;
  response.data = malloc(response.capacity);
  if (!response.data) {
    curl_slist_free_all(headers);
    return -1;
  }

  curl_easy_setopt(reader->curl, CURLOPT_WRITEDATA, &response);

  int retry_count = 0;
  CURLcode res;

  while (retry_count < HTTP_MAX_RETRIES) {
    response.size = 0;
    res = curl_easy_perform(reader->curl);

    if (res == CURLE_OK) {
      long response_code;
      curl_easy_getinfo(reader->curl, CURLINFO_RESPONSE_CODE, &response_code);

      if (response_code == 200 || response_code == 206) {
        break;
      }
    }

    retry_count++;
    if (retry_count < HTTP_MAX_RETRIES) {
      sleep((unsigned int)(2 * retry_count));
    }
  }

  curl_slist_free_all(headers);

  if (res != CURLE_OK || response.size == 0) {
    free(response.data);
    return -1;
  }

  size_t actual_read = (response.size < to_read) ? response.size : to_read;
  memcpy(buffer, response.data, actual_read);
  *bytes_read = actual_read;

  free(response.data);
  return 0;
}

int http_reader_read(http_reader_t *reader, uint8_t *buffer, size_t size,
                     size_t *bytes_read) {
  int result = http_reader_read_at(reader, reader->current_pos, buffer, size,
                                   bytes_read);
  if (result == 0) {
    reader->current_pos += *bytes_read;
  }
  return result;
}

uint64_t http_reader_get_size(http_reader_t *reader) {
  return reader->content_length;
}
