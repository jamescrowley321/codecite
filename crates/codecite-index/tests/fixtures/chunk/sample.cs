using System;

namespace Sample
{
    public interface IVerifier
    {
        bool Verify();
    }

    public enum Grant
    {
        Code,
        Implicit,
    }

    public class Client
    {
        public Client(string issuer)
        {
            Issuer = issuer;
        }

        public string Issuer { get; }

        public bool Validate(string token)
        {
            return token.Length > 0;
        }
    }
}
