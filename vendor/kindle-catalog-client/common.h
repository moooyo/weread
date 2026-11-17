#ifndef WEREAD_CATALOG_CLIENT_COMMON_H
#define WEREAD_CATALOG_CLIENT_COMMON_H

#include <stdio.h>
#include <stdint.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

/* A host fixture may override this at build time. Runtime callers cannot
 * select another document, service, URL, or catalog entry. */
#ifndef ENTRY_DOCUMENT_PATH
#define ENTRY_DOCUMENT_PATH "/mnt/us/documents/weread.sh"
#endif

#endif
