package example;

/** Client documentation. */
@Deprecated
public class Client {
    /** Creates a client. */
    public Client() {}

    public Client(String name) {}

    /** Validates a token. */
    @Deprecated
    public boolean validate(String token) {
        return token != null;
    }

    public boolean validate(int token) { return token > 0; }

    // Not Javadoc.
    public void plain() {} // Trailing comment, not the next method's docs.
    public void next() {}

    /** Nested verifier. */
    interface Verifier {
        /** Checks a token. */
        boolean verify(String token);
    }

    static class Inner {
        public Inner() {}
        void run() {}
    }
}

/** Detached documentation. */

interface Detached {
    void execute();
}

/** Task status. */
enum Status {
    OPEN, CLOSED;

    /** Whether work remains. */
    boolean active() {
        return this == OPEN;
    }
}

record Point(int x, int y) {
    int sum() { return x + y; }
}

@interface Note {
    String value();
}
