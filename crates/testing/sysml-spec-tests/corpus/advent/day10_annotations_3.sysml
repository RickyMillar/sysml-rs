package L10_Annotations {
    part def SantaSleigh {
        //...
        ref item santa {
            comment locale "en_US"
            /* This is a comment with a locale. */

            comment namedComment about greet
            /* This is also a comment -- now with a name, and about "greet". */

            /* One last comment -- next ahead is the textual representation! */

            action greet {
                // We will learn about actions in a later episode
                //...
            }
        }
    }
}
