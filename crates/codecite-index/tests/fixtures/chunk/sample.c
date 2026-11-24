/* C fixture */
int declared(int value);

struct CClient {
    int value;
};

enum Mode { OFF, ON };
union Value { int number; char byte; };
typedef unsigned long Count;

int validate(int value) {
    return value > 0;
}

struct CClient *make_client(void) {
    return 0;
}

char **make_names(void) {
    return 0;
}

typedef struct Pair { int first; int second; } Pair;
